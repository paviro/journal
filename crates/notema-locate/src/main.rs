//! Tiny macOS helper: grab the device's current location via CoreLocation and
//! print it as JSON (`{"latitude":..,"longitude":..,"accuracy":..}`) on success,
//! or a message on stderr with a non-zero exit on failure.
//!
//! This exists as a separate, signed **.app** because a bare command-line binary
//! can no longer obtain CoreLocation authorization on modern macOS (Ventura+):
//! the request is denied with no prompt. The `notema` binary embeds a signed
//! copy of the wrapping `.app`, extracts it, and runs this helper. See
//! `notema-storage`'s macOS device-location provider.

#[cfg(target_os = "macos")]
fn main() {
    match macos::locate() {
        Ok((lat, lon, accuracy)) => match accuracy {
            Some(acc) => println!(r#"{{"latitude":{lat},"longitude":{lon},"accuracy":{acc}}}"#),
            None => println!(r#"{{"latitude":{lat},"longitude":{lon}}}"#),
        },
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("notema-locate only runs on macOS");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AnyThread, DefinedClass, define_class, msg_send};
    use objc2_core_location::{
        CLAuthorizationStatus, CLLocation, CLLocationManager, CLLocationManagerDelegate,
    };
    use objc2_foundation::{NSArray, NSDate, NSDefaultRunLoopMode, NSError, NSRunLoop};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// How long to wait for the user to answer the one-time authorization prompt.
    const AUTH_TIMEOUT: Duration = Duration::from_secs(60);
    /// How long to wait for an actual fix once authorized.
    const FIX_TIMEOUT: Duration = Duration::from_secs(15);
    const DENIED: &str = "Location access is denied — enable it in System Settings → Privacy & Security → Location Services";

    /// A location fix: latitude, longitude, and horizontal accuracy in metres.
    type Fix = (f64, f64, Option<f64>);
    /// Filled in by the delegate on this thread's run loop, drained by the poll
    /// loop. `None` until a fix or error lands.
    type Shared = Arc<Mutex<Option<Result<Fix, String>>>>;

    /// Print a diagnostic line to stderr when `NOTEMA_GPS_DEBUG` is set.
    fn debug(message: impl FnOnce() -> String) {
        if std::env::var_os("NOTEMA_GPS_DEBUG").is_some() {
            eprintln!("[gps] {}", message());
        }
    }

    struct Ivars {
        result: Shared,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "JournalLocationDelegate"]
        #[ivars = Ivars]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl CLLocationManagerDelegate for Delegate {
            #[unsafe(method(locationManager:didUpdateLocations:))]
            fn did_update_locations(
                &self,
                manager: &CLLocationManager,
                locations: &NSArray<CLLocation>,
            ) {
                if let Some(location) = locations.firstObject() {
                    // SAFETY: CoreLocation supplied a live CLLocation in this
                    // delegate callback; both selectors are valid for it.
                    let coord = unsafe { location.coordinate() };
                    // SAFETY: The same callback-owned CLLocation remains live.
                    let accuracy = unsafe { location.horizontalAccuracy() };
                    debug(|| format!("didUpdateLocations {},{}", coord.latitude, coord.longitude));
                    self.finish(Ok((
                        coord.latitude,
                        coord.longitude,
                        // CoreLocation reports a negative accuracy for an invalid fix.
                        (accuracy >= 0.0).then_some(accuracy),
                    )));
                    // SAFETY: CoreLocation supplied this live manager to the
                    // delegate and the selector takes no borrowed arguments.
                    unsafe { manager.stopUpdatingLocation() };
                }
            }

            #[unsafe(method(locationManager:didFailWithError:))]
            fn did_fail(&self, _manager: &CLLocationManager, error: &NSError) {
                // Non-terminal: a transient kCLErrorDenied fires while the prompt
                // is still pending, and kCLErrorLocationUnknown just means "no fix
                // yet". Real denial arrives via didChangeAuthorization, so we wait.
                debug(|| {
                    format!(
                        "didFailWithError code={} {}",
                        error.code(),
                        error.localizedDescription()
                    )
                });
            }

            #[unsafe(method(locationManagerDidChangeAuthorization:))]
            fn did_change_authorization(&self, manager: &CLLocationManager) {
                // SAFETY: CoreLocation supplied a live manager to this callback.
                let status = unsafe { manager.authorizationStatus() };
                debug(|| format!("didChangeAuthorization status={status:?}"));
                match status {
                    CLAuthorizationStatus::NotDetermined => {}
                    CLAuthorizationStatus::Restricted | CLAuthorizationStatus::Denied => {
                        self.finish(Err(DENIED.into()));
                    }
                    // Authorized — make sure updates are flowing (a no-op if already).
                    _ => {
                        // SAFETY: The callback-owned manager is live and
                        // authorization has left the undetermined state.
                        unsafe { manager.startUpdatingLocation() }
                    }
                }
            }
        }
    );

    impl Delegate {
        fn new(result: Shared) -> Retained<Self> {
            let this = Self::alloc().set_ivars(Ivars { result });
            // SAFETY: `this` was allocated as Delegate with all Rust ivars set;
            // NSObject's designated `init` returns the retained object.
            unsafe { msg_send![super(this), init] }
        }

        /// Record the first outcome only; later delegate calls are ignored.
        fn finish(&self, outcome: Result<Fix, String>) {
            let mut slot = self.ivars().result.lock().unwrap();
            if slot.is_none() {
                *slot = Some(outcome);
            }
        }
    }

    pub(crate) fn locate() -> Result<Fix, String> {
        let result: Shared = Arc::new(Mutex::new(None));
        let delegate = Delegate::new(result.clone());
        // SAFETY: `new` is the designated constructor exposed by the generated
        // CoreLocation binding and returns a retained manager.
        let manager = unsafe { CLLocationManager::new() };
        let protocol = ProtocolObject::from_ref(&*delegate);
        // SAFETY: `protocol` is backed by `delegate`, which remains retained for
        // the entire run loop below; the manager is live.
        unsafe { manager.setDelegate(Some(protocol)) };

        // requestWhenInUseAuthorization alone is unreliable; the prompt is raised
        // by actually starting location services. Request authorization (when
        // undetermined) and start updates — that presents the prompt and then
        // delivers fixes once authorized.
        // SAFETY: `manager` is retained for the duration of `locate`.
        match unsafe { manager.authorizationStatus() } {
            CLAuthorizationStatus::Restricted | CLAuthorizationStatus::Denied => {
                return Err(DENIED.into());
            }
            CLAuthorizationStatus::NotDetermined => {
                // SAFETY: The retained manager is live and both selectors take
                // no borrowed Objective-C arguments.
                unsafe {
                    manager.requestWhenInUseAuthorization();
                    manager.startUpdatingLocation();
                }
            }
            _ => {
                // SAFETY: The retained manager is live and already authorized.
                unsafe { manager.startUpdatingLocation() }
            }
        }

        // Pump this thread's run loop in slices until the delegate reports or the
        // deadline passes. While undetermined we wait on the user's prompt answer
        // (AUTH_TIMEOUT); once authorized the fix gets its own shorter window.
        let run_loop = NSRunLoop::currentRunLoop();
        let mut deadline = Instant::now() + AUTH_TIMEOUT;
        let mut authorized = false;
        loop {
            if let Some(outcome) = result.lock().unwrap().take() {
                return outcome;
            }
            if !authorized
                && !matches!(
                    // SAFETY: `manager` remains retained throughout this loop.
                    unsafe { manager.authorizationStatus() },
                    CLAuthorizationStatus::NotDetermined
                )
            {
                authorized = true;
                deadline = Instant::now() + FIX_TIMEOUT;
            }
            if Instant::now() >= deadline {
                return Err("timed out waiting for a location fix from CoreLocation".into());
            }
            let until = NSDate::dateWithTimeIntervalSinceNow(0.2);
            // SAFETY: Foundation exports this process-lifetime run-loop mode
            // constant as a valid NSString reference.
            let mode = unsafe { NSDefaultRunLoopMode };
            run_loop.runMode_beforeDate(mode, &until);
        }
    }
}
