// SPDX-License-Identifier: MPL-2.0
use color_eyre::eyre::{Context, Result};
use dm::objtree::ObjectTree;
use dmm_tools::IconCache;
use std::path::Path;

#[derive(Default)]
pub struct DmContext {
	pub objtree: ObjectTree,
	pub icon_cache: IconCache,
}

impl DmContext {
	pub fn objtree(
		&mut self,
		context: &mut dm::Context,
		game_path: &Path,
		env_file: &str,
	) -> Result<()> {
		let environment = game_path.join(env_file);
		// println!("parsing {}", environment.display());

		if let Some(parent) = environment.parent() {
			self.icon_cache.set_icons_root(parent);
		}

		context.autodetect_config(&environment);
		let pp = dm::preprocessor::Preprocessor::new(&*context, environment)
			.wrap_err("I/O error opening environment")?;
		let indents = dm::indents::IndentProcessor::new(&*context, pp);
		let parser = dm::parser::Parser::new(&*context, indents);
		self.objtree = parser.parse_object_tree();
		Ok(())
	}
}
