//! The deploy kit exists, parses as shell and follows the shutdown and respawn rules; the
//! setup guide covers every step.

use std::path::Path;
use std::process::Command;

fn path(relative: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn read(relative: &str) -> String {
    std::fs::read_to_string(path(relative)).unwrap()
}

#[test]
fn deploy_files_exist_and_parse_as_shell() {
    for file in [
        "deploy/alpine/omron-mcp.initd",
        "deploy/alpine/omron-mcp.confd",
        "scripts/build-release.sh",
    ] {
        assert!(path(file).is_file(), "{file} is missing");

        let status = Command::new("sh")
            .arg("-n")
            .arg(path(file))
            .status()
            .unwrap();

        assert!(status.success(), "sh -n {file}");
    }
}

#[test]
fn init_script_follows_the_shutdown_and_respawn_rules() {
    let initd = read("deploy/alpine/omron-mcp.initd");

    for line in [
        "supervisor=\"supervise-daemon\"",
        "respawn_delay=2",
        "respawn_max=5",
        "retry=\"TERM/60/KILL/5\"",
        "command_user=\"omron:omron\"",
        "command_args=\"serve\"",
        "/var/log/omron-mcp/",
    ] {
        assert!(initd.contains(line), "omron-mcp.initd lacks {line}");
    }
}

#[test]
fn confd_sets_the_port_the_tz_the_data_dir_and_the_memory_caps() {
    let confd = read("deploy/alpine/omron-mcp.confd");

    for line in [
        "export OMRON_MCP_PUBLIC_URL=\"https://omron.example.com\"",
        "export OMRON_MCP_LISTEN=\"127.0.0.1:8082\"",
        "export OMRON_MCP_DATA_DIR=\"/var/lib/omron-mcp\"",
        "export OMRON_MCP_TZ=\"Asia/Jakarta\"",
        "rc_cgroup_settings",
        "memory.high 24M",
        "memory.max 32M",
    ] {
        assert!(confd.contains(line), "omron-mcp.confd lacks {line}");
    }
}

#[test]
fn setup_guide_covers_every_step() {
    let guide = read("docs/setup.md");
    let items = [
        ("the musl build", "scripts/build-release.sh"),
        ("copying the binary", "scp "),
        ("the omron user", "adduser -S -D -H -G omron"),
        ("set-password", "omron-mcp set-password"),
        ("login", "omron-mcp login"),
        ("status", "omron-mcp status"),
        ("starting the service", "rc-service omron-mcp start"),
        ("the health check", "127.0.0.1:8082/healthz"),
        ("the existing tunnel", "No second `cloudflared`"),
        ("the public hostname", "http://127.0.0.1:8082"),
        (
            "no Cloudflare Access",
            "Do **not** put a Cloudflare Access application",
        ),
        (
            "the connector",
            "Settings -> Connectors -> Add custom connector",
        ),
    ];

    for (what, text) in items {
        assert!(guide.contains(text), "setup.md lacks {what}: {text:?}");
    }
}
