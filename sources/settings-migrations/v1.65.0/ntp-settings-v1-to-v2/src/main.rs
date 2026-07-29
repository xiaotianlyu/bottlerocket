// TODO: this migration is not finished. Two things before shipping to real fleets:
//   1. Never actually run — all testing so far was fresh nodes, which skip migration.
//      Need to upgrade a real V1 node and confirm it boots with the right config.
//   2. forward() doesn't inject prefer/minpoll/maxpoll for the link-local IP
//      (169.254.169.123), so upgraded nodes keep their old config and don't get the
//      faster-recovery fix, which is the point of the ticket. See TODO in forward().
use migration_helpers::{migrate, Migration, MigrationData, Result};
use serde_json::Value;
use std::process;

// V1 stores NTP as two flat keys: a list of server URLs plus one shared options list.
const V1_SERVERS: &str = "settings.ntp.time-servers";
const V1_OPTIONS: &str = "settings.ntp.options";

// V2 stores each server as its own map entry under the same prefix, with the
// datastore flattening every field into its own key, e.g.
//   settings.ntp.time-servers.<name>.address
//   settings.ntp.time-servers.<name>.directive
//   settings.ntp.time-servers.<name>.options
const V2_PREFIX: &str = "settings.ntp.time-servers.";

/// NTP moved from a flat server list (V1) to a per-server map (V2).
pub struct NtpSettingsV1ToV2;

impl Migration for NtpSettingsV1ToV2 {
    /// Upgrade: expand the V1 URL list into per-server V2 entries. Each server
    /// is named `server-<index>`, gets `directive = "server"`, and inherits the
    /// shared V1 options.
    fn forward(&mut self, mut input: MigrationData) -> Result<MigrationData> {
        let servers = match input.data.remove(V1_SERVERS) {
            Some(Value::Array(servers)) => servers,
            Some(other) => {
                println!("'{V1_SERVERS}' is not a list ('{other}'), leaving alone");
                return Ok(input);
            }
            None => {
                println!("Found no '{V1_SERVERS}' to migrate on upgrade");
                return Ok(input);
            }
        };

        // Shared options become each server's options; default to empty if unset.
        let options = match input.data.remove(V1_OPTIONS) {
            Some(value) => value,
            None => Value::Array(Vec::new()),
        };

        for (index, address) in servers.into_iter().enumerate() {
            let name = format!("server-{index}");
            input
                .data
                .insert(format!("{V2_PREFIX}{name}.address"), address);
            // TODO: when address is the link-local IP 169.254.169.123, this should
            // set directive = "server" AND add prefer/minpoll/maxpoll to options, so
            // upgraded nodes actually get the recovery fix. Right now every server
            // just inherits the old options unchanged, so the fix is missing on upgrade.
            input.data.insert(
                format!("{V2_PREFIX}{name}.directive"),
                Value::String("server".to_string()),
            );
            input
                .data
                .insert(format!("{V2_PREFIX}{name}.options"), options.clone());
            println!("Migrated NTP server '{name}' to V2");
        }

        Ok(input)
    }

    /// Downgrade: collapse the per-server map back into a flat URL list. The
    /// per-server `directive` (e.g. pool) and per-server options cannot be
    /// represented in V1, so they are dropped.
    fn backward(&mut self, mut input: MigrationData) -> Result<MigrationData> {
        // Collect V2 addresses so we can rebuild the flat list, then drop all V2 keys.
        let mut addresses: Vec<(String, Value)> = Vec::new();
        let v2_keys: Vec<String> = input
            .data
            .keys()
            .filter(|k| k.starts_with(V2_PREFIX))
            .cloned()
            .collect();

        if v2_keys.is_empty() {
            println!("Found no V2 NTP servers to migrate on downgrade");
            return Ok(input);
        }

        for key in v2_keys {
            if let Some(value) = input.data.remove(&key) {
                // key layout: settings.ntp.time-servers.<name>.<field>
                let rest = &key[V2_PREFIX.len()..];
                if let Some((name, field)) = rest.split_once('.') {
                    if field == "address" {
                        addresses.push((name.to_string(), value));
                    } else {
                        println!("Dropping V2-only NTP field '{key}' on downgrade");
                    }
                }
            }
        }

        // Sort by name so the rebuilt list order is deterministic.
        addresses.sort_by(|a, b| a.0.cmp(&b.0));
        let servers: Vec<Value> = addresses.into_iter().map(|(_, addr)| addr).collect();

        input
            .data
            .insert(V1_SERVERS.to_string(), Value::Array(servers));
        println!("Collapsed V2 NTP servers back into '{V1_SERVERS}' on downgrade");

        Ok(input)
    }
}

fn run() -> Result<()> {
    migrate(NtpSettingsV1ToV2)
}

// Returning a Result from main makes it print a Debug representation of the error, but with Snafu
// we have nice Display representations of the error, so we wrap "main" (run) and print any error.
// https://github.com/shepmaster/snafu/issues/110
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        process::exit(1);
    }
}
