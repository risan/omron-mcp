use mcp_starter::ServiceInfo;

pub mod app;
pub mod cache;
pub mod cli;
pub mod store;
pub mod sync;
pub mod tools;

pub static HOME_PAGE: &str = include_str!(concat!(env!("OUT_DIR"), "/home.html"));

pub static SERVICE: ServiceInfo = ServiceInfo {
    command: "omron-mcp",
    display_name: "OMRON",
    env_prefix: "OMRON_MCP_",
    default_listen: "127.0.0.1:8082",
    default_data_dir: "/var/lib/omron-mcp",
    default_tz: "Asia/Jakarta",
    scope: "omron",
    favicon: include_bytes!("../assets/favicon.ico"),
    home_page: HOME_PAGE,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_page_is_a_self_contained_public_page() {
        assert!(HOME_PAGE.starts_with("<!doctype html>"));
        assert!(
            HOME_PAGE
                .contains("<title>omron-mcp: OMRON blood pressure for Claude and ChatGPT</title>")
        );
        assert!(HOME_PAGE.contains(r#"<link rel="icon" href="/favicon.ico">"#));
        assert!(!HOME_PAGE.contains("docs/setup.md"));
        assert!(!HOME_PAGE.contains(".works/"));
        assert!(!HOME_PAGE.to_ascii_lowercase().contains("<script"));
        assert!(!HOME_PAGE.contains("{{"));
    }

    #[test]
    fn home_page_keeps_the_words_of_the_setup_link() {
        assert!(HOME_PAGE.contains("see the setup guide."));
    }

    #[test]
    fn home_page_has_no_relative_links() {
        for link in HOME_PAGE.split(r#"href=""#).skip(1) {
            let target = link.split('"').next().unwrap();

            assert!(
                target.starts_with("https://")
                    || target.starts_with('#')
                    || target == "/favicon.ico",
                "relative link: {target}"
            );
        }
    }

    #[test]
    fn home_page_lists_the_tool() {
        assert!(HOME_PAGE.contains("<td><code>omron_get_blood_pressure</code></td>"));
    }
}
