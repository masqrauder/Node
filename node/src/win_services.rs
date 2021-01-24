// Copyright (c) 2019-2021, MASQ (https://masq.ai) and/or its affiliates. All rights reserved.

use crate::sub_lib::logger::Logger;
use std::env::var;
use std::ffi::OsString;
use std::path::{PathBuf, Path};
use std::process::{Child, Command};
use windows_service::{service_dispatcher, Result};

#[cfg(target_os = "windows")]
define_windows_service!(boot_win_masqd, win_masq_deamon); //boot_win_masqd is bound with Windows services,

#[allow(dead_code)]
#[cfg(target_os = "windows")]
fn win_masq_deamon(_arguments: Vec<OsString>) {
    std::process::exit(spawn_child(initiating_command()
        .arg("--initialization"))
                           .wait()
                           .expect("Deamon failed at its exit")
                           .code()
                           .expect("Option with failing exit status"));
}

fn spawn_child(command: &mut Command) -> Child {
    match command.spawn() {
        Ok(ch) => {
            info!(
                Logger::new("Deamon"),
                "Successfully launched in the background"
            );
            ch
        }
        Err(error) => panic!(
            "Starting MASQNode failed; deamon has stayed down.\n
            ERROR: {}",
            error
        ),
    }
}

fn initiating_command() -> Command {
    let path = path_from_environment();
    let command = Command::new(path);
    command
}

fn path_from_environment() -> PathBuf {
    //this will be probably dealt differently once we incorporate a real installation process
    let os_string_key = OsString::from("MASQ_DEAMON_PATH".to_string());
    let path = PathBuf::from(match var(os_string_key) {
        Ok(path) => path,
        Err(error) => panic!(
            "MASQ: the path for the Deamon from EV is not set properly: {}",
            error
        ),
    });
    path.join("MASQNode.exe")
}

#[cfg(target_os = "windows")]
fn main() -> Result<()> {
    // Register generated `ffi_service_main` with the system and start the service, blocking
    // this thread until the service is stopped.
    service_dispatcher::start("masqd", boot_win_masqd)?;
    Ok(())
}

#[cfg(test)]
#[cfg(target_os = "windows")]
mod tests {
    use super::*;
    use crate::test_utils::logging::{init_test_logging, TestLogHandler};
    use crossbeam_channel::unbounded;
    use std::env::{current_dir, set_var};
    use std::process::Stdio;
    use std::str::{Chars, from_utf8};
    use std::time::Duration;
    use std::{env, thread};
    use std::io::{Error, ErrorKind, BufReader, stdout, BufRead};
    use itertools::Itertools;

    // #[test]
    // fn win_masq_deamon_produces_starting_process() {
    //     init_test_logging();
    //     let build_path = deamon_dev_directory();
    //     let os_string_key = OsString::from("MASQ_DEAMON_PATH");
    //     set_var(os_string_key, build_path);
    //     let (tx, rc) = unbounded();
    //     thread::spawn(move|| {
    //         let id = loop {
    //             match rc.try_recv() {
    //                 Ok(id) => break id,
    //                 Err(_) => {thread::sleep(Duration::from_millis(5));
    //                     continue}
    //             }
    //         };
    //         thread::sleep(Duration::from_millis(100));
    //         let kill_result = kill_deamon_with_its_id(id);
    //         Duration::from_millis(100);
    //     }
    //     );
    //     let mut deamon_handle = spawn_child(initiating_command()
    //         .arg("--initialization")
    //         .stdout(Stdio::piped())
    //         .stderr(Stdio::piped()));
    //     let process_id = deamon_handle.id();
    //     loop {
    //         match tx.try_send(process_id) {
    //             Ok(_) => break,
    //             Err(_) => panic!("We cannot send!")
    //         }
    //     }
    //     let output_handle = deamon_handle.wait_with_output().unwrap();
    //     let stdout = from_utf8(output_handle.stdout.as_slice()).unwrap();
    //     let stderr = from_utf8(output_handle.stderr.as_slice()).unwrap();
    //     TestLogHandler::new()
    //         .exists_log_containing("Deamon: Successfully launched in the background");
    //     assert!(stdout.contains("MASQNode_daemon_rCURRENT"));
    //     assert_eq!(stderr, "blahhhh")
    //
    // }


    #[test]
    fn win_masq_deamon_produces_starting_process() {
        init_test_logging();
        let build_path = deamon_dev_directory();
        let os_string_key = OsString::from("MASQ_DEAMON_PATH");
        set_var(os_string_key, build_path);
        let mut deamon_handle = spawn_child(initiating_command()
            .arg("--initialization")
          .stdout(Stdio::piped()));
        thread::spawn(|| {
            thread::sleep(Duration::from_millis(10));
            kill();
        });
        let output_handle = deamon_handle.wait_with_output().unwrap();
        let stdout = from_utf8(output_handle.stdout.as_slice()).unwrap();
        let stderr = from_utf8(output_handle.stderr.as_slice()).unwrap();
        assert!(stdout.contains("MASQNode_daemon_rCURRENT"));
        TestLogHandler::new()
            .exists_log_containing("Deamon: Successfully launched in the background");

    }

    fn deamon_dev_directory() -> String {
        let env_scan = env::args().next().unwrap();
        let mut iter = env_scan.split("\\");
        let index = iter.position(|s| s == "release" || s == "debug").unwrap();
        //rewriting the iterator to return to the beginning; split does not work reversely
        let iter = env_scan.split("\\");
        let iter_after_cut = iter.take(index + 1);
        let mut str_path = String::new();
        iter_after_cut.for_each(|pc| str_path.push_str(&format!("{}\\", pc)));
        str_path
    }
    fn kill_deamon_with_its_id(id:u32)-> bool {
        let str = &format!("/pid {}",id);
        match Command::new("taskkill")
                .args(&[str,"/F"]).output() {
                Ok(_) => true,
                Err(e) => panic!("Couldn't kill process with pid: {}:{}",id,e)
        }
    }
    fn kill() {
        let mut command = Command::new("taskkill");
        command.args(&["/IM", "MASQNode.exe", "/F"]);
        let _ = command.output().expect("Couldn't kill MASQNode.exe");
    }
}
