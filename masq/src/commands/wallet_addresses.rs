use clap::{App, SubCommand, Arg};
use crate::commands::commands_common::{Command, CommandError, transaction};
use crate::command_context::CommandContext;
use std::any::Any;
use masq_lib::messages::{UiWalletAddressesRequest, UiWalletAddressesResponse};

#[derive(Debug, PartialEq)]
pub struct WalletAddressesCommand{
    db_password: String
}

impl WalletAddressesCommand {
    pub fn new(pieces: Vec<String>) -> Result<Self, String> {
        let matches = match wallet_addresses_subcommand().get_matches_from_safe(pieces) {
            Ok(matches) => matches,
            Err(e) => return Err(format!("{}", e)),
        };
        Ok(Self {
            db_password: matches
                .value_of("db-password")
                .expect("wallet-addresses: Clipy: internal error")
                .to_string(),
        })
    }
}
pub fn wallet_addresses_subcommand()-> App<'static, 'static>{
    SubCommand::with_name("wallet_addresses")
        .about("XXXXXXXXXXXXXXXXXXXXXXXXXX")
        .arg(Arg::with_name ("db-password")
            .help ("XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXt")
            .index (1)
            .required (true)
            .case_insensitive(false)
        )
}

impl Command for WalletAddressesCommand {
    fn execute(&self, context: &mut dyn CommandContext) -> Result<(), CommandError> {
        let input = UiWalletAddressesRequest {
            db_password: self.db_password.clone(),
        };
        let msg: UiWalletAddressesResponse = transaction(input, context, 1000)?;
               writeln!(context.stdout(),
                        "Your consuming wallet address: {}  \
                         Your earning wallet address: {}",
                        msg.consuming_wallet_address,
                        msg.earning_wallet_address).expect("writeln! failed");
               Ok(())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_context::ContextError;
    use crate::command_factory::{CommandFactory, CommandFactoryError, CommandFactoryReal};
    use crate::commands::commands_common::{Command, CommandError};
    use crate::test_utils::mocks::CommandContextMock;
    use masq_lib::messages::{ToMessageBody, UiWalletAddressesResponse, UiWalletAddressesRequest};
    use std::sync::{Arc, Mutex};


    #[test]
    fn testing_command_factory_with_good_command() {
        let subject = CommandFactoryReal::new();

        let result = subject
            .make(vec!["wallet-addresses".to_string(), "bonkers".to_string()])
            .unwrap();

        let wallet_addresse_command: &WalletAddressesCommand = result.as_any().downcast_ref().unwrap();
        assert_eq!(
            wallet_addresse_command,
            &WalletAddressesCommand {
                db_password: "bonkers".to_string(),
            }
        );
    }
    #[test]
    fn testing_command_factory_with_bad_command() {
        let subject = CommandFactoryReal::new();

        let result = subject.make(vec![
            "wallet-addresses".to_string(),
        ]);

        match result {
            Err(CommandFactoryError::CommandSyntax(msg)) => {
                // Note: when run with MASQ/Node/ci/all.sh, msg contains escape sequences for color.
                assert_eq!(
                    msg.contains("The following required arguments were not provided:"),
                    true,
                    "{}",
                    msg
                )
            }
            x => panic!("Expected CommandSyntax error, got {:?}", x),
        }
    }

    #[test]
    fn wallet_address_command_with_password_right() {
        let transact_params_arc = Arc::new(Mutex::new(vec![]));
        let mut context = CommandContextMock::new()
            .transact_params(&transact_params_arc)
            .transact_result(Ok(UiWalletAddressesResponse{ consuming_wallet_address: "0x464654jhkjhk6".to_string(), earning_wallet_address: "0x454654klljkjk".to_string() }.tmb(0)));
        let stdout_arc = context.stdout_arc();
        let stderr_arc = context.stderr_arc();
        let factory = CommandFactoryReal::new();
        let subject = factory
            .make(vec!["wallet-addresses".to_string(), "bonkers".to_string()])
            .unwrap();

        let result = subject.execute(&mut context);

        assert_eq!(result, Ok(()));
        assert_eq!(
            stdout_arc.lock().unwrap().get_string(),
            "Your consuming wallet address: 0x464654jhkjhk6  Your earning wallet address: 0x454654klljkjk\n"
        );
        assert_eq!(stderr_arc.lock().unwrap().get_string(), String::new());
        let transact_params = transact_params_arc.lock().unwrap();
        assert_eq!(
            *transact_params,
            vec![(
                UiWalletAddressesRequest {
                    db_password: "bonkers".to_string(),
                }
                    .tmb(0),
                1000
            )]
        )
    }

    #[test]
    fn wallet_addresses_command_with_no_password_wrong() {
        //works the same if the password is incorrect
        let transact_params_arc = Arc::new(Mutex::new(vec![]));
        let mut context = CommandContextMock::new()
            .transact_params(&transact_params_arc)
            .transact_result( Err(ContextError::PayloadError(4644,"bad bad bad thing".to_string())));
        let stderr_arc = context.stderr_arc();
        let factory = CommandFactoryReal::new();
        let subject = factory.make(vec!["wallet-addresses".to_string(),"some password".to_string()]).unwrap();

        let result = subject.execute(&mut context);

        assert_eq!(result,Err(CommandError::Payload(4644,"bad bad bad thing".to_string())));
        assert_eq!(stderr_arc.lock().unwrap().get_string(), String::new());
        let transact_params = transact_params_arc.lock().unwrap();
        assert_eq!(
            *transact_params,
            vec![(
                UiWalletAddressesRequest {
                    db_password: "some password".to_string(),
                }
                    .tmb(0),
                1000
            )]
        )
    }

    #[test]
    fn wallet_addresses_command_handles_send_failure() {
        let mut context = CommandContextMock::new().transact_result(Err(
            ContextError::ConnectionDropped("tummyache".to_string()),
        ));
        let subject =
            WalletAddressesCommand::new(vec!["wallet-addresses".to_string(), "bonkers".to_string()])
                .unwrap();

        let result = subject.execute(&mut context);

        assert_eq!(
            result,
            Err(CommandError::ConnectionProblem("tummyache".to_string()))
        )
    }
}