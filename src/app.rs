//! The state every request handler shares.

use std::sync::Arc;

use mcp_starter::config::Config;
use mcp_starter::work::Work;

use crate::sync::Omron;
use crate::tools::ToolOptions;

pub struct App {
    pub omron: Omron,
    pub config: Config,
    pub tools: ToolOptions,
    pub work: Arc<Work>,
}

impl App {
    pub fn new(omron: Omron, config: Config, tools: ToolOptions, work: Arc<Work>) -> App {
        App {
            omron,
            config,
            tools,
            work,
        }
    }
}
