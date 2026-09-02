// SPDX-License-Identifier: MPL-2.0
extern crate dreammaker as dm;

pub mod automapper;
pub mod config;
pub mod context;
pub mod encode;
pub mod render;

use crate::{
	automapper::AutomapperConfig,
	config::{MapCategory, MapConfig, ResolvedFlags, ServerConfig},
	context::DmContext,
	render::{MapOutputSpec, ProgressState, create_render_passes, generate_minimap},
};
use color_eyre::eyre::{Context, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{
	path::{Path, PathBuf},
	sync::Arc,
	time::{Duration, Instant},
};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn total_bar_style() -> ProgressStyle {
	ProgressStyle::with_template(
		"{spinner:.green} [{elapsed_precise}] {bar:40.green/white.dim} {pos}/{len} {msg}",
	)
	.expect("invalid progress style template")
	.progress_chars("=>-")
	.tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", ""])
}

fn collect_category_maps<'a>(
	cat: &'a MapCategory,
	out_path: &Path,
	automapper: Option<Arc<AutomapperConfig>>,
) -> impl Iterator<Item = (&'a MapConfig, MapOutputSpec)> {
	let base_map_path = cat.base_map_path();
	let cat_name = cat.name.clone();
	let out_path = out_path.to_owned();
	let direct = cat.maps.iter().map({
		let base_map_path = base_map_path.clone();
		let cat_name = cat_name.clone();
		let out_path = out_path.clone();
		let automapper = automapper.clone();
		move |m| {
			let flags = ResolvedFlags::resolve(m, None, cat);
			let map_dir = out_path.join(&cat_name).join(&m.name);
			let pipes_dir = flags
				.supports_pipes
				.then(|| out_path.join(&cat_name).join("pipes").join(&m.name));
			(m, MapOutputSpec {
				base_map_path: base_map_path.clone(),
				map_dir,
				pipes_dir,
				flags,
				automapper: automapper.clone(),
			})
		}
	});
	let from_subs = cat.subcategories.iter().flat_map({
		let base_map_path = base_map_path.clone();
		let cat_name = cat_name.clone();
		let out_path = out_path.clone();
		let automapper = automapper.clone();
		move |sub| {
			let base_map_path = base_map_path.clone();
			let cat_name = cat_name.clone();
			let out_path = out_path.clone();
			let automapper = automapper.clone();
			let sub_name = sub.name.clone();
			sub.maps.iter().map(move |m| {
				let flags = ResolvedFlags::resolve(m, Some(sub), cat);
				let map_dir = out_path.join(&cat_name).join(&sub_name).join(&m.name);
				let pipes_dir = flags.supports_pipes.then(|| {
					out_path
						.join(&cat_name)
						.join(&sub_name)
						.join("pipes")
						.join(&m.name)
				});
				(m, MapOutputSpec {
					base_map_path: base_map_path.clone(),
					map_dir,
					pipes_dir,
					flags,
					automapper: automapper.clone(),
				})
			})
		}
	});
	direct.chain(from_subs)
}

fn main() -> Result<()> {
	color_eyre::install()?;

	let t_config = Instant::now();
	let config = {
		let config_file = std::fs::read("config.json").wrap_err("failed to read config.json")?;
		serde_json::from_slice::<ServerConfig>(&config_file)
			.wrap_err("failed to parse config.json")?
	};
	let elapsed_config = t_config.elapsed();

	if !config.out_path.exists() {
		std::fs::create_dir_all(&config.out_path).wrap_err_with(|| {
			format!(
				"failed to create output folder at {}",
				config.out_path.display()
			)
		})?;
	}

	let progress = {
		let mp = MultiProgress::new();
		let total = mp.add(ProgressBar::new(0));
		total.set_style(total_bar_style());
		total.set_message("rendering + encoding");
		total.enable_steady_tick(Duration::from_millis(100));
		ProgressState { mp, total }
	};

	// Collect unique (game_path, env_file) pairs in order to avoid redundant
	// env parses
	let mut env_keys: Vec<(PathBuf, String)> = Vec::new();
	for cat in &config.categories {
		let key = (cat.game_path.clone(), cat.env_file.clone());
		if !env_keys.contains(&key) {
			env_keys.push(key);
		}
	}

	let mut elapsed_parse = Duration::ZERO;
	let mut elapsed_render = Duration::ZERO;
	let optimize_options = config.optimize_options();

	for (game_path, env_file) in &env_keys {
		let t_parse = Instant::now();
		let mut dm_ctx = dm::Context::default();
		let mut dm_context = DmContext::default();
		dm_context
			.objtree(&mut dm_ctx, game_path, env_file)
			.wrap_err("failed to setup obj tree")?;
		elapsed_parse += t_parse.elapsed();

		let render_passes = create_render_passes(&config, &dm_ctx.config().map_renderer);

		let mut group_maps = Vec::new();
		for cat in config
			.categories
			.iter()
			.filter(|cat| &cat.game_path == game_path && &cat.env_file == env_file)
		{
			let automapper = cat
				.automapper_config_path
				.as_ref()
				.map(|path| AutomapperConfig::from_file(&cat.game_path.join(path), &cat.game_path))
				.transpose()
				.wrap_err_with(|| {
					format!("failed to load automapper config for category {}", cat.name)
				})?
				.map(Arc::new);
			group_maps.extend(collect_category_maps(cat, &config.out_path, automapper));
		}

		let t_render = Instant::now();
		group_maps.par_iter().for_each(|(map_config, output)| {
			if let Err(err) = generate_minimap(
				map_config,
				&dm_context,
				&render_passes,
				&config,
				&optimize_options,
				output.clone(),
				&progress,
			) {
				progress.total.println(format!(
					"failed to generate minimap for {}: {err}",
					map_config.name
				));
			}
		});
		elapsed_render += t_render.elapsed();
	}

	progress.total.finish_and_clear();
	progress.mp.clear().ok();

	println!("done :)");
	println!("  config load:     {:.2?}", elapsed_config);
	println!("  env parse:       {:.2?}", elapsed_parse);
	println!("  render + encode: {:.2?}", elapsed_render);

	Ok(())
}
