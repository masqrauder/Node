// Copyright (c) 2019-2021, MASQ (https://masq.ai). All rights reserved.

#![cfg (target_os = "macos")]

use crate::daemon::daemonization::daemonizer::{DaemonizerError, DaemonHandle, DaemonHandleFactory};

pub fn platform_daemonize<F: FnOnce() -> Result<(), DaemonizerError>>(daemon_code: F) -> Result<(), DaemonizerError> {
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

pub struct DaemonHandleFactoryReal {}

impl DaemonHandleFactory for DaemonHandleFactoryReal {
    fn make(&self) -> Result<Box<dyn DaemonHandle>, DaemonizerError> {
        unimplemented!()
    }
}

impl DaemonHandleFactoryReal {
    pub fn new() -> Self {
        Self {}
    }
}
