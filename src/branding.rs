//! The SVG files are the source for all runtime icons, rendered at their target size.
use anyhow::{Context, Result, ensure};

pub fn pixels(svg: &[u8], size: u32) -> Result<Vec<u8>> {
    ensure!(size > 0 && size <= 4096, "invalid icon size");
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default())
        .context("failed to parse logo SVG")?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).context("allocate icon pixels")?;
    let scale = size as f32 / tree.size().width().max(tree.size().height());
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // Native icon APIs expect straight alpha; tiny-skia renders premultiplied alpha.
    Ok(pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn both_svg_variants_render_at_native_icon_sizes() {
        for svg in [
            include_bytes!("../assets/logo.svg").as_slice(),
            include_bytes!("../assets/logo-mark.svg").as_slice(),
        ] {
            for size in [16, 22, 32, 64, 72, 256, 1024] {
                let pixels = super::pixels(svg, size).unwrap();
                assert_eq!(pixels.len(), (size * size * 4) as usize);
                assert_eq!(pixels[3], 0, "logo background must remain transparent");
                assert_eq!(pixels[pixels.len() - 1], 0);
                assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[3] != 0));
            }
        }
    }
}
