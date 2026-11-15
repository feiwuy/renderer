//! Display sizes are not render resolutions: keep the scene grid intact and fit its output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub w: usize,
    pub h: usize,
}

impl Size {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w: w.max(1),
            h: h.max(1),
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let parts: Vec<_> = raw.trim().split(['x', 'X', ',']).collect();
        let (w, h) = match parts.as_slice() {
            [n] => {
                let n = n.trim().parse::<usize>().ok()?;
                (n, n)
            }
            [w, h] => (
                w.trim().parse::<usize>().ok()?,
                h.trim().parse::<usize>().ok()?,
            ),
            _ => return None,
        };
        if w == 0 || h == 0 {
            None
        } else {
            Some(Self { w, h })
        }
    }
}

/// One reserved column prevents pending automatic wrap; one reserved row prevents scrolling.
/// Actual terminal rows/columns always win over estimated pixel/cell sizes.
pub fn terminal_bounds(
    requested: Size,
    terminal: Option<Size>,
    pixels: Option<Size>,
    cell: (f32, f32),
    fit: bool,
) -> Size {
    if !fit {
        return requested;
    }
    let mut bounds = requested;
    if let Some(size) = terminal {
        bounds.w = bounds.w.min(size.w.saturating_sub(1).max(1));
        bounds.h = bounds.h.min(size.h.saturating_sub(1).max(1));
    }
    if let Some(px) = pixels {
        let valid = |v: f32, fallback: f32| {
            if v.is_finite() && v > 0.0 {
                v
            } else {
                fallback
            }
        };
        let cw = valid(cell.0, 7.0);
        let ch = valid(cell.1, 14.0);
        bounds.w = bounds.w.min(((px.w as f32 / cw).floor() as usize).max(1));
        bounds.h = bounds.h.min(((px.h as f32 / ch).floor() as usize).max(1));
    }
    Size::new(bounds.w, bounds.h)
}

/// Downsample without changing the scene's aspect ratio; never upscale beyond the source grid.
pub fn contain(source: Size, bounds: Size) -> Size {
    let scale = (bounds.w as f64 / source.w as f64)
        .min(bounds.h as f64 / source.h as f64)
        .min(1.0);
    Size::new(
        (source.w as f64 * scale).floor() as usize,
        (source.h as f64 * scale).floor() as usize,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_sizes_strictly() {
        assert_eq!(Size::parse("329x295"), Some(Size::new(329, 295)));
        assert_eq!(Size::parse(" 128 X 64 "), Some(Size::new(128, 64)));
        assert_eq!(Size::parse("64"), Some(Size::new(64, 64)));
        for bad in ["", "0x64", "128x", "128x64x2", "a,64", "-1x4"] {
            assert_eq!(Size::parse(bad), None, "{bad}");
        }
    }
    #[test]
    fn fits_329_by_295_without_growing_the_grid() {
        let grid = Size::new(128, 64);
        let bounds = terminal_bounds(
            grid,
            Some(Size::new(47, 21)),
            Some(Size::new(329, 295)),
            (7.0, 14.0),
            true,
        );
        assert_eq!(bounds, Size::new(46, 20));
        assert_eq!(contain(grid, bounds), Size::new(40, 20));
    }
    #[test]
    fn never_overrides_real_terminal_capacity() {
        let bounds = terminal_bounds(
            Size::new(128, 64),
            Some(Size::new(30, 15)),
            Some(Size::new(329, 295)),
            (7.0, 14.0),
            true,
        );
        assert_eq!(bounds, Size::new(29, 14));
        assert_eq!(
            terminal_bounds(
                Size::new(128, 64),
                Some(Size::new(2, 2)),
                None,
                (7.0, 14.0),
                true
            ),
            Size::new(1, 1)
        );
    }
    #[test]
    fn leaves_full_size_when_space_is_available() {
        assert_eq!(
            terminal_bounds(
                Size::new(128, 64),
                Some(Size::new(200, 80)),
                None,
                (7.0, 14.0),
                true
            ),
            Size::new(128, 64)
        );
        assert_eq!(
            contain(Size::new(128, 64), Size::new(256, 128)),
            Size::new(128, 64)
        );
    }
    #[test]
    fn fit_can_be_disabled_and_bad_cell_values_are_safe() {
        assert_eq!(
            terminal_bounds(
                Size::new(128, 64),
                Some(Size::new(20, 10)),
                None,
                (7.0, 14.0),
                false
            ),
            Size::new(128, 64)
        );
        assert_eq!(
            terminal_bounds(
                Size::new(128, 64),
                None,
                Some(Size::new(329, 295)),
                (f32::NAN, 0.0),
                true
            ),
            Size::new(47, 21)
        );
    }
}
