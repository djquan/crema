/// Inverse sRGB EOTF (IEC 61966-2-1): linear light [0,1] -> perceptual sRGB [0,1].
pub fn linear_to_srgb(x: f32) -> f32 {
    if x <= 0.0031308 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB EOTF (IEC 61966-2-1): perceptual sRGB [0,1] -> linear light [0,1].
pub fn srgb_to_linear(x: f32) -> f32 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

/// Convert linear sRGB to OKLab (Bjorn Ottosson 2020).
///
/// Returns (L, a, b) where L is in [0,1] for in-gamut colors,
/// a and b are roughly +/-0.3. Chroma = sqrt(a^2 + b^2).
pub fn linear_srgb_to_oklab(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;

    let l_ = l.max(0.0).cbrt();
    let m_ = m.max(0.0).cbrt();
    let s_ = s.max(0.0).cbrt();

    let big_l = 0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_;
    let ok_a = 1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_;
    let ok_b = 0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_;

    (big_l, ok_a, ok_b)
}

/// Approximate maximum OKLab chroma for in-gamut sRGB colors.
/// Actual max is ~0.323 (pure magenta). Rounded up for a clean margin.
pub const OKLAB_MAX_CHROMA: f32 = 0.33;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_roundtrip() {
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            let rt = srgb_to_linear(linear_to_srgb(x));
            assert!((rt - x).abs() < 1e-5, "roundtrip failed at {x}: got {rt}");
        }
    }

    #[test]
    fn srgb_endpoints() {
        assert!((linear_to_srgb(0.0)).abs() < 1e-7);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-7);
        assert!((srgb_to_linear(0.0)).abs() < 1e-7);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-7);
    }

    #[test]
    fn srgb_linear_segment() {
        // Below 0.0031308 should use the linear segment
        let x = 0.001;
        let srgb = linear_to_srgb(x);
        assert!(
            (srgb - 12.92 * x).abs() < 1e-7,
            "linear segment: {srgb} vs {}",
            12.92 * x,
        );
    }

    #[test]
    fn srgb_monotonic() {
        let mut prev = 0.0_f32;
        for i in 1..=1000 {
            let x = i as f32 / 1000.0;
            let y = linear_to_srgb(x);
            assert!(y > prev, "not monotonic at {x}: {y} <= {prev}");
            prev = y;
        }
    }

    #[test]
    fn oklab_black() {
        let (l, a, b) = linear_srgb_to_oklab(0.0, 0.0, 0.0);
        assert!(l.abs() < 1e-6, "black L should be 0, got {l}");
        assert!(a.abs() < 1e-6, "black a should be 0, got {a}");
        assert!(b.abs() < 1e-6, "black b should be 0, got {b}");
    }

    #[test]
    fn oklab_white() {
        let (l, a, b) = linear_srgb_to_oklab(1.0, 1.0, 1.0);
        assert!((l - 1.0).abs() < 0.01, "white L should be ~1.0, got {l}");
        assert!(a.abs() < 0.01, "white a should be ~0, got {a}");
        assert!(b.abs() < 0.01, "white b should be ~0, got {b}");
    }

    #[test]
    fn oklab_gray_is_achromatic() {
        let (_, a, b) = linear_srgb_to_oklab(0.2, 0.2, 0.2);
        let chroma = (a * a + b * b).sqrt();
        assert!(
            chroma < 0.005,
            "gray should have near-zero chroma, got {chroma}"
        );
    }

    #[test]
    fn oklab_saturated_color_has_chroma() {
        let (_, a, b) = linear_srgb_to_oklab(1.0, 0.0, 0.0);
        let chroma = (a * a + b * b).sqrt();
        assert!(
            chroma > 0.2,
            "pure red should have high chroma, got {chroma}"
        );
        assert!(
            chroma < OKLAB_MAX_CHROMA + 0.05,
            "chroma should be below max, got {chroma}"
        );
    }

    #[test]
    fn oklab_l_increases_with_luminance() {
        let (l1, _, _) = linear_srgb_to_oklab(0.1, 0.1, 0.1);
        let (l2, _, _) = linear_srgb_to_oklab(0.5, 0.5, 0.5);
        let (l3, _, _) = linear_srgb_to_oklab(0.9, 0.9, 0.9);
        assert!(l1 < l2, "L should increase: {l1} vs {l2}");
        assert!(l2 < l3, "L should increase: {l2} vs {l3}");
    }

    #[test]
    fn oklab_known_chroma_values() {
        // Verify chroma for sRGB gamut corners against precomputed values.
        let cases: &[(&str, f32, f32, f32, f32)] = &[
            ("red", 1.0, 0.0, 0.0, 0.258),
            ("green", 0.0, 1.0, 0.0, 0.295),
            ("blue", 0.0, 0.0, 1.0, 0.313),
            ("magenta", 1.0, 0.0, 1.0, 0.323),
        ];
        for &(name, r, g, b, expected_chroma) in cases {
            let (_, a, ob) = linear_srgb_to_oklab(r, g, b);
            let chroma = (a * a + ob * ob).sqrt();
            assert!(
                (chroma - expected_chroma).abs() < 0.01,
                "{name}: chroma={chroma}, expected ~{expected_chroma}"
            );
        }
    }

    #[test]
    fn oklab_max_chroma_covers_gamut() {
        // OKLAB_MAX_CHROMA should be >= the chroma of all sRGB gamut corners.
        let corners: &[(f32, f32, f32)] = &[
            (1.0, 0.0, 0.0),
            (0.0, 1.0, 0.0),
            (0.0, 0.0, 1.0),
            (1.0, 1.0, 0.0),
            (1.0, 0.0, 1.0),
            (0.0, 1.0, 1.0),
        ];
        for &(r, g, b) in corners {
            let (_, a, ob) = linear_srgb_to_oklab(r, g, b);
            let chroma = (a * a + ob * ob).sqrt();
            assert!(
                chroma <= OKLAB_MAX_CHROMA,
                "gamut corner ({r},{g},{b}) chroma {chroma} exceeds OKLAB_MAX_CHROMA {}",
                OKLAB_MAX_CHROMA
            );
        }
    }

    #[test]
    fn srgb_continuity_at_breakpoint() {
        // The sRGB transfer function should be continuous at the breakpoint.
        let bp = 0.0031308_f32;
        let below = linear_to_srgb(bp - 1e-7);
        let above = linear_to_srgb(bp + 1e-7);
        let gap = (above - below).abs();
        assert!(
            gap < 1e-4,
            "sRGB should be continuous at breakpoint: gap={gap}"
        );
    }

    #[test]
    fn srgb_inverse_linear_segment() {
        // Below 0.04045 perceptual should use the linear segment
        let x = 0.03;
        let linear = srgb_to_linear(x);
        assert!(
            (linear - x / 12.92).abs() < 1e-7,
            "inverse linear segment: {linear} vs {}",
            x / 12.92
        );
    }
}
