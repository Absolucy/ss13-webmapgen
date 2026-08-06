// SPDX-License-Identifier: MPL-2.0
use crate::{config::ServerConfig, render::GeneratedMinimap};
use color_eyre::eyre::{Context, Result, eyre};
use indicatif::ProgressBar;

pub fn generate_minimap_image(
	minimap: GeneratedMinimap,
	config: &ServerConfig,
	optimize_options: &oxipng::Options,
	encode_bar: &ProgressBar,
) -> Result<()> {
	let GeneratedMinimap {
		map_dir,
		name,
		z,
		image,
	} = minimap;
	std::fs::create_dir_all(&map_dir)
		.wrap_err_with(|| format!("failed to create output directory {}", map_dir.display()))?;
	if config.generate_webp || config.webp_only {
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
		std::fs::write(map_dir.join(format!("{name}-{z}.webp")), webp)
			.wrap_err("failed to write webp")?;
	}

	if !config.webp_only {
		let png = oxipng::RawImage::new(
			image.width,
			image.height,
			oxipng::ColorType::RGBA,
			oxipng::BitDepth::Eight,
			bytemuck::cast_vec(image.data.into_raw_vec_and_offset().0),
		)
		.wrap_err("failed to create raw png image")?;
		let optimized_png = png
			.create_optimized_png(optimize_options)
			.wrap_err("failed to optimize png image")?;
		std::fs::write(map_dir.join(format!("{name}-{z}.png")), optimized_png)
			.wrap_err("failed to write optimized png")?;
	}
	encode_bar.inc(1);
	Ok(())
}
