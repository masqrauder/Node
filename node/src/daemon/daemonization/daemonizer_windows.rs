// Copyright (c) 2019-2021, MASQ (https://masq.ai). All rights reserved.

#![cfg (target_os = "windows")]

use crate::daemon::daemonization::daemonizer::{DaemonizerError, DaemonHandle, DaemonHandleFactory};
use windows_service::{Error};
use std::ffi::{OsString};
use windows_service::service_control_handler::{ServiceStatusHandle, ServiceControlHandlerResult};
use windows_service::service::{ServiceControl, ServiceStatus};

define_windows_service!(masqd, masqd_fn);

pub fn platform_daemonize<F: FnOnce() -> Result<(), DaemonizerError>>(daemon_code: F) -> Result<(), DaemonizerError> {
    daemon_code ()?;
    Ok(())
}

fn masqd_fn (arguments: Vec<OsString>) {
    unimplemented!()
}

pub struct DaemonHandleReal {

}

impl DaemonHandle for DaemonHandleReal {
    fn signal_termination(&self) {
        unimplemented!()
    }

    fn finish_termination(&self) {
        unimplemented!()
    }
}

impl DaemonHandleReal {
    pub fn new() -> Self {
        unimplemented!()
    }
}

pub struct DaemonHandleFactoryReal {
    service_registrar: Box<dyn ServiceRegistrar>,
}

impl DaemonHandleFactory for DaemonHandleFactoryReal {
    fn make(&self) -> Result<Box<dyn DaemonHandle>, DaemonizerError> {
        self.service_registrar.register ("masqd");
        unimplemented!()
    }
}

impl DaemonHandleFactoryReal {
    pub fn new() -> Self {
        Self {
            service_registrar: Box::new (ServiceRegistrarReal::new()),
        }
    }
}

trait ServiceRegistrar {
    fn register(&self, service_name: &str) -> Result<ServiceStatusHandle, windows_service::Error>;
    fn handle_event (&self, control: ServiceControl) -> ServiceControlHandlerResult;
}

struct ServiceRegistrarReal {

}

impl ServiceRegistrar for ServiceRegistrarReal {
    fn register(&self, service_name: &str) -> Result<ServiceStatusHandle, Error> {
        unimplemented!()
    }

    fn handle_event(&self, control: ServiceControl) -> ServiceControlHandlerResult {
        unimplemented!()
    }
}

impl ServiceRegistrarReal {
    fn new () -> Self {
        Self {}
    }
}

trait StatusHandleWrapper {
    fn set_service_status(&self, service_status: ServiceStatus) -> Result<(), Error>;
}

struct StatusHandleReal {
    status_handle: ServiceStatusHandle,
}

impl StatusHandleWrapper for StatusHandleReal {
    fn set_service_status(&self, service_status: ServiceStatus) -> Result<(), Error> {
        self.status_handle.set_service_status(service_status)
    }
}

impl StatusHandleReal {
    fn new (status_handle: ServiceStatusHandle) -> Self {
        Self {status_handle}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::{Mutex, Arc};

    #[derive (Clone, PartialEq, Debug)]
    struct DaemonHandleMock {

    }

    impl DaemonHandle for DaemonHandleMock {
        fn signal_termination(&self) {
            unimplemented!()
        }

        fn finish_termination(&self) {
            unimplemented!()
        }
    }

    impl DaemonHandleMock {
        fn new () -> Self {
            Self {}
        }
    }

    struct ServiceRegistrarMock {
        register_params: Arc<Mutex<Vec<String>>>,
        register_results: RefCell<Vec<Result<ServiceStatusHandle, Error>>>,
        handle_event_params: Arc<Mutex<Vec<ServiceControl>>>,
        handle_event_results: RefCell<Vec<ServiceControlHandlerResult>>,
    }

    impl ServiceRegistrar for ServiceRegistrarMock {
        fn register(&self, service_name: &str) -> Result<ServiceStatusHandle, Error> {
            self.register_params.lock().unwrap().push (service_name.to_string());
            self.register_results.borrow_mut().remove(0)
        }

        fn handle_event(&self, control: ServiceControl) -> ServiceControlHandlerResult {
            self.handle_event_params.lock().unwrap().push (control);
            self.handle_event_results.borrow_mut().remove (0)
        }
    }

    impl ServiceRegistrarMock {
        fn new () -> Self {
            Self {
                register_params: Arc::new(Mutex::new(vec![])),
                register_results: RefCell::new(vec![]),
                handle_event_params: Arc::new(Mutex::new(vec![])),
                handle_event_results: RefCell::new(vec![])
            }
        }

        fn register_params (mut self, params: &Arc<Mutex<Vec<String>>>) -> Self {
            self.register_params = params.clone();
            self
        }

        fn register_result (self, result: Result<ServiceStatusHandle, Error>) -> Self {
            self.register_results.borrow_mut().push (result);
            self
        }

        fn handle_event_params (mut self, params: &Arc<Mutex<Vec<ServiceControl>>>) -> Self {
            self.handle_event_params = params.clone();
            self
        }

        fn handle_event_result (self, result: ServiceControlHandlerResult) -> Self {
            self.handle_event_results.borrow_mut().push (result);
            self
        }
    }

    #[test]
    fn handle_factory_constructor_registers_service () {
        let daemon_handle = DaemonHandleMock::new();
        let service_status_params_arc = Arc::new (Mutex::new (vec![]));
        let service_status_handle = what?
        let service_registrar = ServiceRegistrarMock::new()
            .register_params (&service_status_params_arc)
            .register_result (Ok(service_status_handle));
        let mut subject = DaemonHandleFactoryReal::new();
        subject.service_registrar = Box::new (service_registrar);

        let result = subject.make ();

        assert_eq! (result.is_ok(), true);
        let service_status_params = service_status_params_arc.lock().unwrap();
        assert_eq! (*service_status_params, vec!["masqd".to_string()])
    }
}
