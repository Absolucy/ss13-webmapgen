// SPDX-License-Identifier: MPL-2.0
use crate::config::ServerConfig;
use color_eyre::eyre::{Context, Result};
use dm::objtree::ObjectTree;
use dmm_tools::IconCache;
use std::sync::atomic::AtomicIsize;

#[derive(Default)]
pub struct DmContext {
	pub dm_context: dm::Context,
	pub objtree: ObjectTree,
	pub icon_cache: IconCache,
	pub exit_status: AtomicIsize,
	pub parallel: bool,
}

impl DmContext {
	pub fn objtree(&mut self, config: &ServerConfig) -> Result<()> {
		let environment = config.game_path.join(&config.dme_name);
		println!("parsing {}", environment.display());

		if let Some(parent) = environment.parent() {
			self.icon_cache.set_icons_root(parent);
		}

		self.dm_context.autodetect_config(&environment);
		let pp = dm::preprocessor::Preprocessor::new(&self.dm_context, environment)
			.wrap_err("I/O error opening environment")?;
		let indents = dm::indents::IndentProcessor::new(&self.dm_context, pp);
		let parser = dm::parser::Parser::new(&self.dm_context, indents);
		self.objtree = parser.parse_object_tree();
		Ok(())
	}
}
