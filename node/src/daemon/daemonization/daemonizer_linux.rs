// Copyright (c) 2019-2021, MASQ (https://masq.ai). All rights reserved.

#![cfg (target_os = "linux")]

use crate::daemon::daemonization::daemonizer::{DaemonizerError, DaemonHandle};

pub fn daemonize() -> Result<DaemonHandle, DaemonizerError> {
    unimplemented!()
}
