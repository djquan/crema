# Image Adjustment Pipeline

This document describes every image processing decision in crema's editing pipeline: what each adjustment does, the math behind it, why that math was chosen, and how it compares to industry tools like Adobe Lightroom, Capture One, and Apple Photos.

If you have never edited a photo before, the [Primer](#primer-how-digital-photos-work) section will get you up to speed.

---

## Table of Contents

- [Primer: How Digital Photos Work](#primer-how-digital-photos-work)
- [Pipeline Overview](#pipeline-overview)
- [Working Color Space](#working-color-space)
- [White Balance](#1-white-balance)
- [Exposure](#2-exposure)
- [Tone Curve](#3-tone-curve-contrast-highlights-shadows-blacks)
- [Vibrance](#4-vibrance)
- [Saturation](#5-saturation)
- [Crop](#6-crop)
- [Auto Enhance](#auto-enhance)
- [Display Conversion](#display-conversion)
- [Color Science Utilities](#color-science-utilities)
- [Comparison Matrix](#comparison-with-other-software)

---

## Primer: How Digital Photos Work

A digital photo is a grid of pixels. Each pixel stores three numbers: how much red, green, and blue light it contains. Mix all three at full intensity and you get white; zero on all three gives you black.

Camera sensors capture light intensity linearly: twice as many photons produce twice the signal. But human eyes are logarithmic; we are far more sensitive to differences in shadows than in highlights. The sRGB standard bridges this gap with a *gamma curve* (technically a piecewise transfer function) that redistributes values so that "50% brightness on screen" looks like perceptual mid-gray, not the washed-out result you would get from a linear encoding.

```
           Human perception          Camera sensor

Bright  ┃ ░░░░░░░░░░░░░░░░░░░░    ┃ ██████████████████████████████
        ┃ ░░░░░░░░░░░░░░░░░░      ┃ ████████████████████████████
        ┃ ░░░░░░░░░░░░░░░░        ┃ ██████████████████████████
Mid     ┃ ░░░░░░░░░░░░░░          ┃ ███████████████████
        ┃ ░░░░░░░░░░░░            ┃ ████████████████
        ┃ ░░░░░░░░░░              ┃ ████████████
        ┃ ░░░░░░░░                ┃ ████████
Dark    ┃ ░░░░░░                  ┃ ████
        ┗━━━━━━━━━━━━━━━━━━━━━    ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
          Equal perceptual steps       Equal light intensity steps
```

Photo editors work in *linear light* internally (matching the camera sensor) and apply the perceptual curve only at the very end for display. This keeps the math physically correct: doubling exposure means doubling every pixel value.

**Key vocabulary:**

| Term | Meaning |
|------|---------|
| **Linear light** | Pixel values proportional to physical light intensity. 0.5 = half the photons of 1.0 |
| **sRGB gamma** | The standard perceptual encoding for monitors. ~2.2 power curve with a linear toe |
| **Scene-referred** | Values can exceed 1.0 (a bright sky might be 3.0). Clipping happens only at display time |
| **EV (exposure value)** | A doubling/halving of light. +1 EV = twice as bright; -1 EV = half as bright |
| **Luminance (Y)** | Perceived brightness of a pixel: `Y = 0.2126R + 0.7152G + 0.0722B` (Rec. 709 weights) |
| **Chroma** | How colorful a pixel is, independent of brightness |
| **Hue** | The "name" of a color (red, orange, teal, etc.) |

---

## Pipeline Overview

Every edit in crema is *non-destructive*. The original image is never modified. Instead, an `EditParams` struct stores slider values, and the pipeline re-applies them from scratch each time a slider moves.

```
                    EditParams
                        │
  Raw/JPEG ──> ImageBuf │ (linear f32 RGB, scene-referred)
                   │    │
                   ▼    ▼
              ┌─────────────┐
              │ White Balance│  Color temperature + tint correction
              └──────┬──────┘
                     ▼
              ┌─────────────┐
              │  Exposure    │  Global brightness (EV stops)
              └──────┬──────┘
                     ▼
              ┌─────────────┐
              │  Tone Curve  │  Contrast, highlights, shadows, blacks
              └──────┬──────┘  (4096-entry LUT in perceptual space)
                     ▼
              ┌─────────────┐
              │  Vibrance    │  Selective saturation (OKLab-based)
              └──────┬──────┘
                     ▼
              ┌─────────────┐
              │  Saturation  │  Uniform saturation
              └──────┬──────┘
                     ▼
              ┌─────────────┐
              │    Crop      │  Normalized region extraction
              └──────┬──────┘
                     ▼
              ┌─────────────┐
              │   Display    │  linear f32 → sRGB u8 (LUT, 4096 entries)
              └─────────────┘
```

**Why this order?** White balance must come first because all downstream math assumes neutral lighting. Exposure sets the overall level before tonal adjustments redistribute it. The tone curve reshapes the brightness distribution. Color adjustments (vibrance, saturation) come after tonal work so they operate on the final luminance relationships. Crop is last because it is purely geometric.

This matches the order used by Lightroom, Capture One, and most professional editors.

Each module implements a `ProcessingModule` trait with one method:

```rust
fn process_cpu(&self, input: ImageBuf, params: &EditParams) -> Result<ImageBuf>
```

Every module has an **identity fast path**: if its slider is at the default value, it returns the input unchanged with zero computation.

---

## Working Color Space

All pixel math happens in **linear sRGB** (IEC 61966-2-1). Pixel values are stored as `f32` in a flat interleaved layout: `[R, G, B, R, G, B, ...]`, three floats per pixel.

Values are **scene-referred**: they can exceed 1.0 (bright highlights from RAW files) or go below 0.0 (temporary results from aggressive adjustments). Clamping to [0, 1] happens only at the final display conversion.

**Why linear sRGB?**

- Physically correct: multiplying by 2.0 actually doubles the light
- Matches camera sensor output (after demosaicing)
- Required for correct white balance, exposure, and luminance math
- Simple: no ICC profile management needed for sRGB displays (the vast majority)

Lightroom and Capture One also work in linear space internally (Lightroom uses a ProPhoto-like gamut called "Melissa RGB"; Capture One uses a custom linear space). Apple Photos uses a simpler perceptual pipeline. Crema chose sRGB gamut because it matches the target display gamut; wider gamut handling (P3, Rec. 2020) is a future consideration.

---

## 1. White Balance

**File:** `crates/crema-core/src/pipeline/modules/white_balance.rs`
**Slider range:** Temperature 1667K–25000K (default 5500K), Tint -150 to +150 (default 0)
**Identity:** temp = 5500 AND tint = 0

### What It Does

White balance removes color casts caused by the light source. A photo taken under tungsten bulbs looks orange; under fluorescent lights it looks green. The white balance slider shifts the overall color so that neutral objects (white walls, gray cards) appear neutral.

Temperature controls the blue-orange axis (matching the physical color of heated objects, the "Planckian locus"). Tint controls the green-magenta axis perpendicular to the locus.

### The Math

Crema uses a **Bradford chromatic adaptation transform (CAT)**, the same algorithm specified by the ICC v4 profile standard and used in Lightroom and Capture One.

The processing chain for each pixel:

```
linear sRGB → XYZ → Bradford cone space → adapt → XYZ → linear sRGB
```

This is collapsed into a single **3x3 matrix** that is precomputed once per slider change and applied per-pixel.

**Step 1: Temperature to chromaticity**

The Planckian locus (the curve of colors produced by ideal heated objects) is approximated using the Kang et al. (2002) polynomials, the standard reference used in color science:

```
If temp <= 4000K:
    x = -0.2661239e9/T³ - 0.2343589e6/T² + 0.8776956e3/T + 0.179910

If temp > 4000K:
    x = -3.0258469e9/T³ + 2.1070379e6/T² + 0.2226347e3/T + 0.240390

y = polynomial in x (three ranges: <=2222K, 2222-4000K, >4000K)
```

**Step 2: Tint as perpendicular offset**

Tint shifts the chromaticity perpendicular to the Planckian locus in CIE 1960 UCS (uniform chromaticity scale). This matches Lightroom's tint behavior:

```
Duv = tint / 3000.0

Tangent direction = normalize(locus(T+50) - locus(T-50))
Normal direction  = (-tangent_y, tangent_x)

Final chromaticity = locus(T) + Duv * normal
```

Positive tint pushes toward magenta (below the locus); negative pushes toward green (above).

**Step 3: Bradford adaptation**

```
Source white  = XYZ of D55 (5500K reference)
Target white  = XYZ of the slider's (temp, tint) chromaticity

Bradford matrix transforms XYZ to a "cone response" space:

    ┌       ┐   ┌                              ┐ ┌     ┐
    │ ρ_src │   │  0.8951   0.2664  -0.1614    │ │ X_s │
    │ γ_src │ = │ -0.7502   1.7135   0.0367    │ │ Y_s │
    │ β_src │   │  0.0389  -0.0685   1.0296    │ │ Z_s │
    └       ┘   └                              ┘ └     ┘

Scale factors = target_cone / source_cone (per channel)

Final matrix = XYZ_to_sRGB * Bradford_inv * diag(scale) * Bradford * sRGB_to_XYZ
```

This 3x3 result is applied to every pixel as a single matrix-vector multiply.

### Why Bradford?

Bradford is the ICC v4 standard for chromatic adaptation. It models human cone cell responses more accurately than simpler methods (Von Kries, XYZ scaling). The perceptual result is that whites look naturally white across a wide range of color temperatures without introducing unwanted tints in saturated colors.

**Comparison:**
- **Lightroom, Capture One:** Bradford CAT (same approach)
- **Apple Photos:** Simplified per-channel multiplier (less accurate for extreme shifts)
- **darktable:** Bradford by default, with options for CAT16 (from CIECAM16)

---

## 2. Exposure

**File:** `crates/crema-core/src/pipeline/modules/exposure.rs`
**Slider range:** -5.0 to +5.0 EV (default 0.0)
**Identity:** exposure = 0.0

### What It Does

Exposure simulates changing the camera's sensor exposure time. +1 EV doubles all light; -1 EV halves it. This is the most intuitive brightness control: it matches the physical behavior of opening or closing a camera aperture by one stop.

### The Math

```
multiplier = 2^exposure

For each pixel:
    R = R * multiplier
    G = G * multiplier
    B = B * multiplier
```

No clamping is applied. Values can exceed 1.0 (the tone curve and display conversion handle this later).

### Why Powers of Two?

Photography measures light in stops, where each stop is a doubling. The `2^EV` formula means the slider is calibrated in stops, matching every camera meter and every other photo editor. A +1.0 slider value means exactly one stop brighter.

**Comparison:**
- **Lightroom:** Same `2^EV` formula
- **Capture One:** Same `2^EV` formula
- **Apple Photos:** Uses a perceptual brightness curve (nonlinear, not physically based)

---

## 3. Tone Curve (Contrast, Highlights, Shadows, Blacks)

**File:** `crates/crema-core/src/pipeline/modules/tone_curve.rs`
**Slider ranges:** All -100 to +100 (default 0)
**Identity:** all four sliders at 0

### What It Does

The tone curve reshapes the brightness distribution of the image without changing the overall exposure. It has four controls that target different brightness regions:

| Control | Target Region | Positive Effect | Negative Effect |
|---------|--------------|-----------------|-----------------|
| **Contrast** | Full range | S-curve (darks darker, lights lighter) | Inverse S (flatter) |
| **Highlights** | Bright areas | Brighter highlights | Recovered/pulled down |
| **Shadows** | Dark areas | Lifted/brighter shadows | Crushed/darker shadows |
| **Blacks** | Deepest shadows | Lifted black point + softer rolloff | Crushed black point |

### The Math

The tone curve is implemented as a **4096-entry lookup table (LUT)** built in perceptual space (gamma 2.2 approximation). Working in perceptual space means that equal slider increments produce roughly equal visual changes, which makes the sliders feel linear to the user.

**Zone layout in perceptual space [0, 1]:**

```
    0.0          0.10    0.15     0.35              0.65      0.90     1.0
     │            │       │        │                 │         │        │
     ▼            ▼       ▼        ▼                 ▼         ▼        ▼
    ┌─────────────┬───────┬────────┬─────────────────┬─────────┬────────┐
    │  Blacks     │ Blend │Shadows │    Midtones     │Highlights│ Blend  │
    │  [0, 0.15]  │       │[.10,.35]│  (identity)    │[.65,.90] │        │
    └─────────────┴───────┴────────┴─────────────────┴─────────┴────────┘
                    5% feather regions for C¹ continuity
```

**Contrast: generalized Reinhard S-curve**

```
a = 3^(contrast / 100)

output = x^a / (x^a + (1 - x)^a)
```

When `a = 1`, this is the identity. When `a > 1`, it produces an S-curve that darkens shadows and brightens highlights. When `a < 1`, it compresses contrast (useful for HDR-like flat looks).

This particular S-curve formula (also called a "sigmoid power curve") is preferred over a simple gamma or polynomial because:
- It always passes through (0, 0), (0.5, 0.5), and (1, 1)
- It is symmetric around the midpoint
- It never clips or overshoots
- A single parameter controls the entire shape

**Highlights and Shadows: zone power curves**

```
gamma = 3^(-slider / 100)

Within each zone:
    normalized = (value - zone_lo) / (zone_hi - zone_lo)
    output = zone_lo + normalized^gamma * (zone_hi - zone_lo)
```

Positive slider values produce gamma < 1 (lift/brighten). Negative values produce gamma > 1 (crush/darken). The `3^x` base was chosen empirically to match the visual feel of Lightroom's sliders.

Zone boundaries blend smoothly via **Hermite smoothstep** (`t * t * (3 - 2t)`) over a 5% feather region, ensuring C1 continuity (no visible banding at zone edges).

**Blacks: lift + power curve**

```
gamma = 3^(-blacks / 100)
lift  = max(blacks, 0) * 0.10
range = 0.15 - lift

normalized = (value / 0.15).clamp(0, 1)
output = lift + normalized^gamma * range
```

Positive blacks lifts the black point (no pure black in the image, a "faded film" look) while softening the shadow rolloff. Negative blacks crushes toward deeper black.

**HDR extension (values above 1.0)**

Scene-referred images from RAW files can have pixel values well above 1.0. The tone curve LUT only covers [0, 1], so values above 1.0 are handled by linear extrapolation using the slope at the top of the LUT:

```
slope = (lut[4095] - lut[4094]) * 4095
output = lut[4095] + slope * (value - 1.0)
```

This maintains C1 continuity at the 1.0 boundary and preserves highlight detail in HDR content.

**Per-pixel application: luminance-ratio scaling**

The LUT maps perceptual luminance to adjusted luminance. But applying a luminance change naively (per-channel) would shift hue. Instead, crema computes a single scale factor from the luminance ratio and applies it to all three channels:

```
Y = 0.2126R + 0.7152G + 0.0722B

perceptual_y = Y^(1/2.2)             // to perceptual space
new_y        = lut_lerp(perceptual_y) // look up in the 4096-entry LUT
new_linear_y = new_y^2.2             // back to linear

scale = new_linear_y / Y
R, G, B *= scale
```

This preserves the hue of every pixel (the R:G:B ratio stays the same).

**Monotonicity enforcement:** After building the LUT, a single pass ensures `lut[i] >= lut[i-1]`. Extreme combinations of sliders could theoretically produce a non-monotonic curve, which would invert tonal relationships and look visually wrong.

### Comparison

- **Lightroom:** Similar zone-based approach. Uses parametric curves with four control points. Exact formulas are proprietary, but the user-facing behavior is very similar.
- **Capture One:** Uses a "Levels" model with separate highlight/shadow curves. More granular (separate "high dynamic range" tool), but conceptually the same.
- **Apple Photos:** Simpler "Brilliance" and "Highlights/Shadows" sliders. Less precise zone control.
- **darktable:** Offers both a parametric curve (like Lightroom) and a "filmic RGB" tone mapper for scene-referred HDR data. The filmic approach is more physically motivated but harder to use.

---

## 4. Vibrance

**File:** `crates/crema-core/src/pipeline/modules/vibrance.rs`
**Slider range:** -100 to +100 (default 0)
**Identity:** vibrance = 0

### What It Does

Vibrance is *intelligent* saturation. Instead of boosting all colors equally (which quickly makes skin tones look sunburned and skies look electric), vibrance targets colors that are already muted and leaves already-saturated colors mostly alone. It also protects skin tones.

This makes it the go-to tool for making landscapes "pop" without ruining faces.

### The Math

**Step 1: Measure pixel saturation using OKLab**

```
(L, a, b) = linear_srgb_to_oklab(R, G, B)
chroma = sqrt(a² + b²)
sat = (chroma / 0.33).clamp(0, 1)
```

OKLab is a perceptually uniform color space (designed by Bjorn Ottosson in 2020). Using it for saturation measurement means "equally saturated-looking" colors get equal `sat` values, regardless of hue. This is a significant improvement over HSV or HSL saturation, which overestimates yellows and underestimates blues.

0.33 is the approximate maximum chroma for in-gamut sRGB colors (achieved by pure magenta).

**Step 2: Compute selective effect strength**

```
strength = vibrance_slider / 100.0

effect = strength * (1.0 - sign(strength) * sat)
effect = max(effect, -1.0)
```

When `strength > 0` (boosting):
- A pixel with `sat = 0` (gray) gets the full effect
- A pixel with `sat = 1` (maximally saturated) gets zero effect
- Everything in between is proportional

When `strength < 0` (desaturating):
- The selectivity inverts: saturated pixels are affected more
- At -100%, all pixels are fully desaturated regardless of starting saturation

This selectivity formula matches the convention used by SweetFX/ReShade, a widely-used post-processing framework.

**Step 3: Skin tone protection**

Skin tones across all ethnicities fall in a narrow hue range (roughly 5 to 55 degrees in HSV, centered on orange/peach). The vibrance module detects skin-tone pixels and reduces the effect by up to 70%:

```
skin_factor = skin_tone_weight(R, G, B)
effect *= 1.0 - skin_factor * 0.7
```

The skin tone detector works in sRGB-gamma-encoded HSV:

```
Hue range:   350 to 85 degrees (wraps through 0/360)
Ramp in:     smoothstep over [350, 5]    (15 degree ramp)
Full plateau: [5, 55]                     (50 degree plateau)
Ramp out:    smoothstep over [55, 85]    (30 degree ramp)
```

The ramps use Hermite smoothstep (`t * t * (3 - 2t)`) for C1-continuous transitions, preventing visible hard edges where protection starts or stops.

The hue calculation uses `.rem_euclid(6.0)` instead of `% 6.0` for the HSV sector computation, which correctly handles negative intermediate values (a defensive fix for edge cases with very dark or near-black pixels).

**Step 4: Apply to pixel**

```
Y = 0.2126R + 0.7152G + 0.0722B
R = (Y + (1 + effect) * (R - Y)).max(0)
G = (Y + (1 + effect) * (G - Y)).max(0)
B = (Y + (1 + effect) * (B - Y)).max(0)
```

This blends each channel toward or away from the luminance value, preserving brightness while changing saturation.

### Comparison

- **Lightroom:** Very similar behavior. Adobe's exact formula is proprietary but produces comparable results. Lightroom also uses skin tone protection, though the detection method is undisclosed.
- **Capture One:** Calls this "Saturation" in their color editor. Offers per-channel vibrance, which is more granular but harder to use.
- **Apple Photos:** Has a "Vibrance" slider. Simpler algorithm without OKLab; uses HSL-based saturation measurement.
- **darktable:** "Color balance RGB" module offers a "vibrance" parameter using JzCzhz (a newer perceptual space). Slightly more accurate than OKLab for wide-gamut work.

---

## 5. Saturation

**File:** `crates/crema-core/src/pipeline/modules/saturation.rs`
**Slider range:** -100 to +100 (default 0)
**Identity:** saturation = 0

### What It Does

Saturation uniformly increases or decreases the colorfulness of every pixel by the same amount, regardless of how colorful it already is. At -100% the image becomes grayscale. At +100% color deviations from luminance are doubled.

Most photographers prefer vibrance for general use. Saturation is the blunt instrument for when you want a specific look (full desaturation for black-and-white, heavy saturation for a hyper-vivid style).

### The Math

```
blend = 1.0 + (saturation / 100.0)

Y = 0.2126R + 0.7152G + 0.0722B
R = (Y + blend * (R - Y)).max(0)
G = (Y + blend * (G - Y)).max(0)
B = (Y + blend * (B - Y)).max(0)
```

`blend = 0` at -100% (full grayscale). `blend = 2` at +100% (doubled deviation). The formula linearly interpolates/extrapolates between the luminance value and the original channel value.

The `.max(0)` clamp prevents negative values from extreme desaturation of already-low channels.

### Comparison

- **Lightroom, Capture One, Apple Photos:** All use essentially the same formula. Saturation is one of the simplest and most standardized adjustments.
- The luminance weights (0.2126, 0.7152, 0.0722) are the Rec. 709 standard, matching sRGB's definition of luminance.

---

## 6. Crop

**File:** `crates/crema-core/src/pipeline/modules/crop.rs`
**Parameters:** crop_x, crop_y (offset), crop_w, crop_h (size), all normalized [0, 1]
**Identity:** x=0, y=0, w=1, h=1

### What It Does

Extracts a rectangular region from the image. Parameters are normalized so that (0, 0, 1, 1) means "the entire image" and (0.25, 0.25, 0.5, 0.5) means "the center quarter."

### The Math

```
src_x = (crop_x * width).clamp(0, width - 1)
src_y = (crop_y * height).clamp(0, height - 1)
dst_w = (crop_w * width).max(1).min(width - src_x)
dst_h = (crop_h * height).max(1).min(height - src_y)
```

Direct scanline copy; no resampling or interpolation. The output `ImageBuf` has the cropped dimensions.

---

## Auto Enhance

**File:** `crates/crema-core/src/pipeline/auto_enhance.rs`

### What It Does

Analyzes the image histogram and produces a set of `EditParams` that corrects common exposure, white balance, and tonal problems. The goal is "make this photo look right with one click," similar to Lightroom's "Auto" button or Apple Photos' "Auto" wand.

### The Algorithm

The analysis runs on the 2048px preview image (sub-millisecond) in a single pass through all pixels.

**1. Build histogram in perceptual space**

All thresholds are defined in sRGB-gamma space because that matches human perception of "how bright something looks." The conversion uses the exact sRGB EOTF (not a gamma 2.2 approximation):

```
Y_linear  = 0.2126R + 0.7152G + 0.0722B
Y_display = linear_to_srgb(Y_linear)
```

Percentiles extracted: p1, p5, p10, p50, p95, p99.

**2. Exposure correction**

Target perceptual mid-gray = 0.461, which is `linear_to_srgb(0.18)`. The value 0.18 (18% reflectance) is the universal photographic mid-gray, the brightness of a standard gray card.

```
raw_ev = log2(0.461 / p50) * 0.45    // 45% correction strength
if |raw_ev| < 0.07: raw_ev = 0       // dead zone for near-correct images
ev = raw_ev.clamp(-2.0, 2.0)
```

The 45% strength prevents overcorrection. A photo that is two stops dark gets corrected by roughly one stop, which looks natural rather than aggressively "fixed."

**3. Feedforward exposure simulation**

Before computing highlight/shadow corrections, the algorithm simulates the effect of the exposure adjustment on the percentiles:

```
corrected_p = linear_to_srgb(srgb_to_linear(p) * 2^ev)
```

This prevents the common auto-enhance bug where the algorithm lifts exposure AND lifts shadows, producing a washed-out result.

**4. Highlight recovery**

```
if p95_corrected > 0.58:
    t = sqrt((p95_corrected - 0.58) / 0.42)
    highlights = -(t * 100).clamp(0, 100)
```

The square root produces gentler correction for mild clipping and stronger correction as clipping becomes severe. The 0.58 threshold is conservative; full highlight recovery would start at 0.85+, but earlier intervention preserves more detail.

**5. Shadow lift**

```
if p10_corrected < 0.38:
    t = sqrt((0.38 - p10_corrected) / 0.38)
    shadows = (t * 70).clamp(0, 70)
```

Same square root scaling. Capped at 70 (not 100) to avoid the "HDR look" of excessively lifted shadows.

**6. Blacks counterbalance**

```
if shadows > 5.0:
    blacks = -min(shadows * 0.3, 25.0)
```

When shadows are lifted, crushing the very deepest blacks slightly prevents the image from looking flat. This is a standard darkroom technique ("adding black point after opening shadows").

**7. Contrast boost for flat images**

```
spread = p95_corrected - p5_corrected
if spread < 0.5:
    contrast = ((0.5 - spread) / 0.5 * 20).clamp(0, 20)
```

Images with compressed tonal range (overcast scenes, haze) get a mild contrast boost. Maximum 20, which is subtle.

**8. White balance via neutral pixel detection**

The algorithm searches for "nearly gray" pixels using OKLab:

```
Candidate pixel: ok_l > 0.3 AND ok_l < 0.85 AND chroma < 0.04
```

If at least 2% of pixels qualify:

```
Temperature from R/B imbalance:
    rb_log = log2(avg_r / avg_b)
    if |rb_log| > 0.05:
        temp = (5500 - rb_log * 1500).clamp(3000, 9000)

Tint from OKLab a-channel:
    if |avg_ok_a| > 0.003:
        tint = (-avg_ok_a * 1000).clamp(-30, 30)
```

This is a "gray world" assumption: in a typical scene, neutral objects should average to gray. The OKLab filtering step is critical; without it, scenes dominated by a single color (green forest, blue sky) would be incorrectly "corrected."

**9. Vibrance**

```
vibrance = ((1.0 - avg_saturation) * 25.0).clamp(0, 25)
```

Muted scenes (low average saturation) get a bigger boost. Already-vibrant scenes get zero. Maximum 25 is conservative to avoid over-processing.

### Comparison

- **Lightroom Auto:** More sophisticated; uses machine learning trained on photographer-edited images. Better at artistic intent, but less predictable.
- **Capture One Auto:** Similar histogram-based approach. Tends to be more aggressive with contrast.
- **Apple Photos Auto:** Very conservative. Focuses mainly on exposure and white balance.
- **darktable Auto:** Exposure-only auto; no auto white balance or tonal adjustment.

---

## Display Conversion

**File:** `crates/crema-core/src/image_buf.rs` (functions `to_rgba_u8_srgb`, `linear_to_srgb_u8`)

### What It Does

Converts the linear f32 pipeline output to sRGB-gamma u8 pixels for display. This is the *only* place gamma encoding happens.

### The Math

A **4096-entry lookup table** maps linear [0, 1] to sRGB u8 [0, 255]. The LUT is built once using the exact sRGB transfer function:

```
if v <= 0.0031308:
    srgb = 12.92 * v
else:
    srgb = 1.055 * v^(1/2.4) - 0.055

u8 = round(srgb * 255)
```

At lookup time, **linear interpolation** between bracketing LUT entries produces sub-code-value accuracy:

```
idx_f = v * 4095.0
i0 = floor(idx_f)
frac = idx_f - i0
result = lut[i0] + frac * (lut[i0+1] - lut[i0])
```

Values are clamped to [0, 1] before lookup. This is where scene-referred values above 1.0 are clipped to display white.

### Why a LUT?

The sRGB formula involves `powf(1/2.4)`, which is expensive per-pixel. With a 4096-entry LUT and linear interpolation, the maximum error is less than 1 code value (out of 255) across the entire range, while being an order of magnitude faster. This matters when converting a 24-megapixel image every time a slider moves.

---

## Color Science Utilities

**File:** `crates/crema-core/src/color.rs`

### sRGB Transfer Function (IEC 61966-2-1)

The sRGB standard defines a piecewise function with a linear segment near black and a power curve above:

```
Linear → Perceptual (inverse EOTF):
    x <= 0.0031308:  y = 12.92 * x
    x >  0.0031308:  y = 1.055 * x^(1/2.4) - 0.055

Perceptual → Linear (EOTF):
    x <= 0.04045:    y = x / 12.92
    x >  0.04045:    y = ((x + 0.055) / 1.055)^2.4
```

The linear toe segment prevents a near-infinite slope at black, which would amplify noise. The two segments meet at the breakpoint with C0 continuity (matching values) and near-C1 continuity (matching slopes).

The exponent is 2.4, not 2.2. The "gamma 2.2" shorthand is a common approximation; the actual sRGB curve is slightly different, especially in the shadows. Crema uses the exact formula everywhere.

### OKLab Color Space (Ottosson 2020)

OKLab is a perceptually uniform color space. "Perceptually uniform" means that a given numerical distance in OKLab corresponds to the same visual difference regardless of where you are in the color space. This is critical for vibrance (measuring "how saturated does this look?") and auto-enhance (finding "nearly gray" pixels).

```
Linear sRGB → LMS (cone response):
    l = 0.4122214·R + 0.5363325·G + 0.0514460·B
    m = 0.2119035·R + 0.6806995·G + 0.1073970·B
    s = 0.0883025·R + 0.2817189·G + 0.6299787·B

Cube root (approximating perceptual response):
    l' = cbrt(max(l, 0))
    m' = cbrt(max(m, 0))
    s' = cbrt(max(s, 0))

LMS' → OKLab:
    L  = 0.2104542·l' + 0.7936178·m' - 0.0040720·s'
    a  = 1.9779985·l' - 2.4285922·m' + 0.4505937·s'
    b  = 0.0259040·l' + 0.7827718·m' - 0.8086758·s'
```

L is lightness [0, 1]. a is the green-red axis. b is the blue-yellow axis. Chroma = `sqrt(a² + b²)`.

The maximum chroma for in-gamut sRGB colors is approximately 0.323 (pure magenta). The constant `OKLAB_MAX_CHROMA = 0.33` provides a small margin above this.

**Why OKLab over CIELAB?** CIELAB (the older standard) has known problems: it overestimates the chroma of blues and has hue non-linearity in the blue-purple region. OKLab was specifically designed to fix these issues. It is now the recommended perceptual space in CSS Color Level 4 and is used by modern design tools.

---

## Comparison with Other Software

| Feature | crema | Lightroom | Capture One | Apple Photos | darktable |
|---------|-------|-----------|-------------|--------------|-----------|
| **Working space** | Linear sRGB | Linear "Melissa RGB" (ProPhoto-like) | Custom linear | Perceptual | Linear Rec. 2020 (scene-referred) |
| **WB method** | Bradford CAT | Bradford CAT | Bradford CAT | Per-channel multiply | Bradford or CAT16 |
| **Exposure** | 2^EV | 2^EV | 2^EV | Perceptual curve | 2^EV |
| **Tone curve** | 4096 LUT, zone-based | Parametric curve | Levels + curves | Simple sliders | Filmic RGB or parametric |
| **Vibrance detection** | OKLab chroma | Proprietary | HSL-based | HSL-based | JzCzhz chroma |
| **Skin protection** | HSV hue + smoothstep | Yes (undisclosed method) | Per-channel control | No | No |
| **Auto enhance** | Histogram percentiles + gray world | ML-trained model | Histogram-based | Conservative auto | Exposure only |
| **Bit depth** | f32 throughout | 16-bit + f32 hybrid | 16-bit integer | 8/16-bit | f32 throughout |
| **Gamut** | sRGB | ProPhoto-like | Custom wide | sRGB/P3 | Rec. 2020 |

### Best Practices (General)

These principles guide crema's pipeline and are shared by most professional editors:

1. **Work in linear light.** Multiplying pixels in gamma-encoded space produces incorrect results (dark haloes, hue shifts). Every serious editor works in linear space for math and applies gamma only at display time.

2. **Preserve hue during tonal adjustments.** The tone curve scales all channels by the same luminance ratio, keeping the R:G:B ratio intact. Lightroom calls this "hue preservation"; it prevents the well-known problem of orange highlights turning yellow when pulled down.

3. **Use perceptually uniform spaces for color measurement.** HSV saturation is not perceptually uniform (a "50% saturated" yellow looks far more vivid than a "50% saturated" blue). OKLab or CIELAB-based chroma gives consistent results across all hues.

4. **Order matters.** White balance before exposure before tone before color before crop. Each step assumes the previous steps have been applied. Reordering would produce different (and usually worse) results.

5. **Scene-referred throughout.** Never clip intermediate values to [0, 1]. A pixel at 1.5 after exposure might be pulled back to 0.9 by highlight recovery. Premature clipping would destroy that information.

6. **Use LUTs for expensive per-pixel operations.** The sRGB transfer function involves `powf`, which is slow. A 4096-entry LUT with linear interpolation trades negligible memory (16KB) for major speedups on megapixel images.

7. **Smooth transitions.** Zone boundaries in the tone curve use smoothstep feathering. Skin tone ramps use smoothstep. Hard transitions create visible banding or halo artifacts that look cheap.

8. **Conservative auto-enhancement.** It is better to under-correct than over-correct. Users can always push sliders further; undoing an aggressive auto result is frustrating. The 45% exposure strength and capped slider ranges reflect this philosophy.
