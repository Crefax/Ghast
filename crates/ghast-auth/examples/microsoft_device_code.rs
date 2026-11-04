//! Microsoft account login with the device-code flow.
//!
//! ```text
//! cargo run -p ghast-auth --features microsoft --example microsoft_device_code -- \
//!     <azure|live> <client-id> <cache-file>
//! ```
//!
//! On the first run it prints a URL and a code; sign in there and the example prints the
//! Minecraft profile. The tokens are stored in `<cache-file>` (mode 0600 on Unix), so later runs
//! log in without asking. This talks to the real Microsoft, Xbox and Minecraft services; it is
//! never run by the tests.

use std::process::ExitCode;

use ghast_auth::microsoft::{MicrosoftAuth, MicrosoftConfig, TokenCache};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let usage = "usage: microsoft_device_code <azure|live> <client-id> <cache-file>";
    let mut args = std::env::args().skip(1);
    let flavor = args.next().ok_or(usage)?;
    let client_id = args.next().ok_or(usage)?;
    let cache = TokenCache::new(args.next().ok_or(usage)?);

    let config = match flavor.as_str() {
        "azure" => MicrosoftConfig::azure_ad(client_id),
        "live" => MicrosoftConfig::live(client_id),
        _ => return Err(usage.into()),
    };
    let auth = MicrosoftAuth::new(config)?;
    let session = auth
        .authenticate(Some(&cache), |code| match &code.message {
            Some(message) => println!("{message}"),
            None => println!(
                "Open {} and enter the code {}",
                code.verification_uri, code.user_code
            ),
        })
        .await?;
    println!(
        "logged in as {} ({}), token valid until {:?}",
        session.profile.name, session.profile.id, session.expires_at
    );
    Ok(())
}
