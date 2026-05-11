// SPDX-License-Identifier: MPL-2.0
use crate::{config::ServerConfig, render::GeneratedMinimap, util::thread_safe_print};
use color_eyre::eyre::{Context, Result, eyre};
use std::time::Instant;

pub fn generate_minimap_image(
	minimap: GeneratedMinimap,
	config: &ServerConfig,
	optimize_options: &oxipng::Options,
) -> Result<()> {
	let GeneratedMinimap { name, z, image } = minimap;
	let mut start = Instant::now();
	if config.generate_webp {
		// lossless produces smaller files than lossy even at the same quality setting
		let raw: &[u8] = bytemuck::cast_slice(
			image
				.data
				.as_slice()
				.ok_or_else(|| eyre!("image data is not contiguous"))?,
		);
		let webp = webp::Encoder::from_rgba(raw, image.width, image.height)
			.encode_lossless()
			.to_vec();
		std::fs::write(config.out_path.join(format!("{name}-{z}.webp")), webp)
			.wrap_err("failed to write webp")?;
		thread_safe_print(format!(
			"{name}-{z} webp done in {:.2} seconds",
			start.elapsed().as_secs_f64()
		));
		start = Instant::now();
	}

	let png = oxipng::RawImage::new(
		image.width,
		image.height,
		oxipng::ColorType::RGBA,
		oxipng::BitDepth::Eight,
		bytemuck::cast_vec(image.data.into_raw_vec()),
	)
	.wrap_err("failed to create raw png image")?;
	let optimized_png = png
		.create_optimized_png(optimize_options)
		.wrap_err("failed to optimize png image")?;
	std::fs::write(
		config.out_path.join(format!("{name}-{z}.png")),
		optimized_png,
	)
	.wrap_err("failed to write optimized png")?;
	thread_safe_print(format!(
		"{name}-{z} png done in {:.2} seconds",
		start.elapsed().as_secs_f64()
	));
	Ok(())
}
