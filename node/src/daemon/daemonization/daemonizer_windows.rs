// Copyright (c) 2019-2021, MASQ (https://masq.ai). All rights reserved.

#![cfg (target_os = "windows")]

use crate::daemon::daemonization::daemonizer::{DaemonizerError, DaemonHandle};

pub fn daemonize() -> Result<DaemonHandle, DaemonizerError> {
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
