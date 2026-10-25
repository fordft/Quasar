//! Colour themes: three seeds per mode, and a contrast, from which the frame
//! derives every surface in CSS. Native paints only the chrome rung, so that
//! one rung is derived here with the same oklab mix.
use serde::Deserialize;

pub type Rgb = [u8; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seeds {
    pub bg: Rgb,
    pub fg: Rgb,
    pub accent: Rgb,
}

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub light: Seeds,
    pub dark: Seeds,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub light: Seeds,
    pub dark: Seeds,
    pub from: Option<String>,
}

pub const DEFAULT: &str = "zephium";
pub const CUSTOM: &str = "custom";
pub const DEFAULT_CONTRAST: u8 = 50;
pub const MAX_CUSTOM_BYTES: usize = 320;

/// The graphite window ground native has always painted, kept exact.
const GRAPHITE_CHROME_LIGHT: Rgb = [230, 230, 233];
const GRAPHITE_CHROME_DARK: Rgb = [31, 31, 34];

/// How far the chrome rung sits from the ground toward the text, at the
/// default contrast. Mirrors `--color-chrome` in `styles/axes/palette.css`.
const CHROME_STEP_LIGHT: f64 = 0.045;
const CHROME_STEP_DARK: f64 = 0.029;

const fn hex(value: u32) -> Rgb {
    [(value >> 16) as u8, (value >> 8) as u8, value as u8]
}

const fn seeds(bg: u32, fg: u32, accent: u32) -> Seeds {
    Seeds {
        bg: hex(bg),
        fg: hex(fg),
        accent: hex(accent),
    }
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "zephium",
        name: "Quasar",
        light: seeds(0xf1f1f3, 0x1c1c1f, 0x222226),
        dark: seeds(0x1a1a1d, 0xf4f4f6, 0xededf0),
    },
    Preset {
        id: "paper",
        name: "Paper",
        light: seeds(0xf6f4ef, 0x23211d, 0x3a62d9),
        dark: seeds(0x1c1b19, 0xeeebe4, 0x8fa8f2),
    },
    Preset {
        id: "midnight",
        name: "Midnight",
        light: seeds(0xeef1f7, 0x141a2c, 0x3460de),
        dark: seeds(0x0f1322, 0xe5e9f4, 0x7ea3ff),
    },
    Preset {
        id: "sand",
        name: "Sand",
        light: seeds(0xefe9de, 0x2d2720, 0xa95b2a),
        dark: seeds(0x221e1a, 0xece3d5, 0xdf9862),
    },
    Preset {
        id: "nord",
        name: "Nord",
        light: seeds(0xeceff4, 0x2e3440, 0x5e81ac),
        dark: seeds(0x2e3440, 0xeceff4, 0x88c0d0),
    },
    Preset {
        id: "catppuccin",
        name: "Catppuccin",
        light: seeds(0xeff1f5, 0x4c4f69, 0x8839ef),
        dark: seeds(0x1e1e2e, 0xcdd6f4, 0xcba6f7),
    },
    Preset {
        id: "dracula",
        name: "Dracula",
        light: seeds(0xfffbeb, 0x1f1f1f, 0x644ac9),
        dark: seeds(0x282a36, 0xf8f8f2, 0xbd93f9),
    },
    Preset {
        id: "gruvbox",
        name: "Gruvbox",
        light: seeds(0xfbf1c7, 0x3c3836, 0xaf3a03),
        dark: seeds(0x282828, 0xebdbb2, 0xfe8019),
    },
    Preset {
        id: "rose-pine",
        name: "Rosé Pine",
        light: seeds(0xfaf4ed, 0x575279, 0xd7827e),
        dark: seeds(0x191724, 0xe0def4, 0xebbcba),
    },
    Preset {
        id: "solarized",
        name: "Solarized",
        light: seeds(0xfdf6e3, 0x586e75, 0x268bd2),
        dark: seeds(0x002b36, 0x93a1a1, 0x268bd2),
    },
    Preset {
        id: "tokyo-night",
        name: "Tokyo Night",
        light: seeds(0xe1e2e7, 0x343b58, 0x2e7de9),
        dark: seeds(0x1a1b26, 0xc0caf5, 0x7aa2f7),
    },
    Preset {
        id: "everforest",
        name: "Everforest",
        light: seeds(0xfdf6e3, 0x5c6a72, 0x8da101),
        dark: seeds(0x2d353b, 0xd3c6aa, 0xa7c080),
    },
    Preset {
        id: "github",
        name: "GitHub",
        light: seeds(0xf6f8fa, 0x1f2328, 0x0969da),
        dark: seeds(0x0d1117, 0xe6edf3, 0x4493f8),
    },
];

pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.id == id)
}

pub fn theme_allowed(value: &str) -> bool {
    value == CUSTOM || preset(value).is_some()
}

fn parse_hex(value: &str) -> Option<Rgb> {
    let digits = value.strip_prefix('#')?;
    if digits.len() != 6
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some(hex(value))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeedsJson {
    bg: String,
    fg: String,
    accent: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeJson {
    light: SeedsJson,
    dark: SeedsJson,
    #[serde(default)]
    from: Option<String>,
}

impl SeedsJson {
    fn seeds(&self) -> Option<Seeds> {
        Some(Seeds {
            bg: parse_hex(&self.bg)?,
            fg: parse_hex(&self.fg)?,
            accent: parse_hex(&self.accent)?,
        })
    }
}

/// A custom theme as stored: lowercase `#rrggbb` seeds per mode and, when
/// it was forked, the preset it came from. Contrast is its own setting.
pub fn parse_custom(value: &str) -> Option<Theme> {
    if value.len() > MAX_CUSTOM_BYTES {
        return None;
    }
    let json: ThemeJson = serde_json::from_str(value).ok()?;
    if json.from.as_deref().is_some_and(|id| preset(id).is_none()) {
        return None;
    }
    Some(Theme {
        light: json.light.seeds()?,
        dark: json.dark.seeds()?,
        from: json.from,
    })
}

/// The theme's own contrast, as `ui.theme-contrast` stores it.
pub fn contrast_allowed(value: &str) -> bool {
    value
        .parse::<u8>()
        .is_ok_and(|contrast| contrast <= 100 && value == contrast.to_string())
}

/// The seeds and contrast the stored settings paint, or nothing while they
/// leave the interface as graphite.
fn resolve(
    theme: &str,
    custom: Option<&str>,
    contrast: u8,
    raised_contrast: bool,
) -> Option<(Theme, u8)> {
    let themed = contrast != DEFAULT_CONTRAST || raised_contrast;
    let resolved = match theme {
        CUSTOM => custom.and_then(parse_custom),
        id => preset(id)
            .filter(|preset| preset.id != DEFAULT || themed)
            .map(|preset| Theme {
                light: preset.light,
                dark: preset.dark,
                from: None,
            }),
    }?;
    Some((
        resolved,
        if raised_contrast {
            100
        } else {
            contrast.min(100)
        },
    ))
}

/// Whether the stored theme leaves the interface as graphite, the default.
pub fn is_default(theme: &str, custom: Option<&str>, contrast: u8, raised_contrast: bool) -> bool {
    resolve(theme, custom, contrast, raised_contrast).is_none()
}

/// The window's chrome rung for the stored theme. The untouched default is
/// graphite, exactly as before themes existed.
pub fn chrome(
    theme: &str,
    custom: Option<&str>,
    contrast: u8,
    raised_contrast: bool,
    light: bool,
) -> Rgb {
    let Some((theme, contrast)) = resolve(theme, custom, contrast, raised_contrast) else {
        return if light {
            GRAPHITE_CHROME_LIGHT
        } else {
            GRAPHITE_CHROME_DARK
        };
    };
    let (seeds, step) = if light {
        (theme.light, CHROME_STEP_LIGHT)
    } else {
        (theme.dark, CHROME_STEP_DARK)
    };
    mix(seeds.fg, seeds.bg, step * spread(contrast))
}

/// The rung multiplier, `--seed-k` in CSS: 0.5 at no contrast, 1.5 at full.
fn spread(contrast: u8) -> f64 {
    0.5 + f64::from(contrast) / 100.0
}

/// `color-mix(in oklab, a <amount>, b)` for opaque sRGB colours.
pub fn mix(a: Rgb, b: Rgb, amount: f64) -> Rgb {
    let (a, b) = (oklab(a), oklab(b));
    let blend = |i: usize| a[i] * amount + b[i] * (1.0 - amount);
    srgb([blend(0), blend(1), blend(2)])
}

fn oklab(rgb: Rgb) -> [f64; 3] {
    let linear = |channel: u8| {
        let c = f64::from(channel) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b] = rgb.map(linear);
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    ]
}

fn srgb([lightness, a, b]: [f64; 3]) -> Rgb {
    let l = (lightness + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m = (lightness - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s = (lightness - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
    let encode = |c: f64| {
        let c = if c <= 0.003_130_8 {
            12.92 * c
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (c.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    [
        encode(4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s),
        encode(-1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s),
        encode(-0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701 * s),
    ]
}

fn css(rgb: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// The stored theme with a new accent in each mode, as `ui.theme-custom`
/// stores it; a preset is forked, so it remembers where it came from.
pub fn with_accent(theme: &str, custom: Option<&str>, (light, dark): (Rgb, Rgb)) -> Option<String> {
    let mut base = match theme {
        CUSTOM => parse_custom(custom?)?,
        id => {
            let preset = preset(id)?;
            Theme {
                light: preset.light,
                dark: preset.dark,
                from: Some(preset.id.to_owned()),
            }
        }
    };
    base.light.accent = light;
    base.dark.accent = dark;
    let seeds = |seeds: &Seeds| serde_json::json!({ "bg": css(seeds.bg), "fg": css(seeds.fg), "accent": css(seeds.accent) });
    let mut json = serde_json::json!({ "light": seeds(&base.light), "dark": seeds(&base.dark) });
    if let Some(from) = base.from {
        json["from"] = from.into();
    }
    Some(json.to_string())
}

/// The presets as the frame reads them, with the ground native paints for
/// each at the default contrast so the frame can check its CSS against it.
pub fn presets_json() -> String {
    let seeds = |seeds: &Seeds| serde_json::json!({ "bg": css(seeds.bg), "fg": css(seeds.fg), "accent": css(seeds.accent) });
    let presets: Vec<_> = PRESETS
        .iter()
        .map(|preset| {
            serde_json::json!({
                "id": preset.id,
                "name": preset.name,
                "light": seeds(&preset.light),
                "dark": seeds(&preset.dark),
                "ground": {
                    "light": css(chrome(preset.id, None, DEFAULT_CONTRAST, false, true)),
                    "dark": css(chrome(preset.id, None, DEFAULT_CONTRAST, false, false)),
                },
            })
        })
        .collect();
    serde_json::to_string_pretty(&presets).map_or_else(|_| "[]\n".into(), |json| json + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NORD: &str = r##"{"light":{"bg":"#eceff4","fg":"#2e3440","accent":"#5e81ac"},"dark":{"bg":"#2e3440","fg":"#eceff4","accent":"#88c0d0"},"from":"nord"}"##;
    const D: u8 = DEFAULT_CONTRAST;

    #[test]
    fn custom_themes_hold_strict_seeds() {
        let theme = parse_custom(NORD).expect("valid");
        assert_eq!(theme.dark.accent, [0x88, 0xc0, 0xd0]);
        assert_eq!(theme.from.as_deref(), Some("nord"));
        assert!(parse_custom(&NORD.replace(r#","from":"nord""#, "")).is_some());
        for invalid in [
            NORD.replace("#eceff4", "#ECEFF4"),
            NORD.replace("#eceff4", "#ecef4"),
            NORD.replace("#eceff4", "eceff4"),
            NORD.replace("#eceff4", "#eceff4ff"),
            NORD.replace("#eceff4", "red"),
            NORD.replace("#eceff4", "url(x)"),
            NORD.replace("\"from\":\"nord\"", "\"from\":\"custom\""),
            NORD.replace("\"from\":\"nord\"", "\"from\":\"nord\",\"contrast\":50"),
            NORD.replace("\"from\":\"nord\"", "\"from\":\"nord\",\"css\":\"body{}\""),
            NORD.replace(",\"accent\":\"#5e81ac\"", ""),
            String::new(),
            "null".into(),
            format!("{NORD}{}", " ".repeat(MAX_CUSTOM_BYTES)),
        ] {
            assert!(parse_custom(&invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn theme_ids_are_presets_or_custom_and_contrast_is_bounded() {
        assert!(theme_allowed("zephium"));
        assert!(theme_allowed("rose-pine"));
        assert!(theme_allowed("custom"));
        for invalid in ["", "Nord", "solarised", "custom ", "../nord"] {
            assert!(!theme_allowed(invalid));
        }
        let mut ids: Vec<_> = PRESETS.iter().map(|preset| preset.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), PRESETS.len());
        for value in ["0", "50", "100"] {
            assert!(contrast_allowed(value));
        }
        for value in ["", "-1", "101", "050", "5.5", " 5"] {
            assert!(!contrast_allowed(value), "{value:?}");
        }
    }

    #[test]
    fn the_default_ground_is_unchanged_graphite() {
        assert_eq!(chrome("zephium", None, D, false, false), [31, 31, 34]);
        assert_eq!(chrome("zephium", None, D, false, true), [230, 230, 233]);
        assert_eq!(
            chrome("custom", Some("not json"), D, false, true),
            [230, 230, 233]
        );
        assert!(is_default("zephium", None, D, false));
        assert!(!is_default("zephium", None, D, true));
        assert!(!is_default("zephium", None, 70, false));
        assert!(!is_default("custom", Some(NORD), D, false));
        // Derived from Zephium's own seeds it lands where graphite is.
        let derived = chrome("zephium", None, D, true, false);
        assert!(derived[0].abs_diff(31) <= 5, "{derived:?}");
    }

    #[test]
    fn the_ground_follows_seeds_mode_and_contrast() {
        let dark = chrome("nord", None, D, false, false);
        assert!(dark[2] > dark[0], "a Nord ground stays blue: {dark:?}");
        let light = chrome("nord", None, D, false, true);
        assert!(
            light[0] < 0xec,
            "light chrome steps toward the text: {light:?}"
        );
        let low = chrome("nord", None, 0, false, false);
        let high = chrome("nord", None, 100, false, false);
        assert!(low[0] < dark[0] && dark[0] < high[0]);
        assert_eq!(chrome("nord", None, 0, true, false), high);
        assert_eq!(chrome("custom", Some(NORD), D, false, false), dark);
    }

    #[test]
    fn oklab_mixing_round_trips_and_matches_css() {
        for rgb in [
            [0, 0, 0],
            [255, 255, 255],
            [0x5e, 0x81, 0xac],
            [0x1a, 0x1a, 0x1d],
        ] {
            assert_eq!(mix(rgb, [0, 0, 0], 1.0), rgb);
        }
        // color-mix(in oklab, #ffffff 50%, #000000) is oklab(0.5 0 0).
        assert_eq!(mix([255, 255, 255], [0, 0, 0], 0.5), [99, 99, 99]);
    }

    #[test]
    fn exported_presets_carry_seeds_and_grounds() {
        let json: serde_json::Value = serde_json::from_str(&presets_json()).unwrap();
        assert_eq!(json.as_array().unwrap().len(), PRESETS.len());
        assert_eq!(json[0]["id"], "zephium");
        assert_eq!(json[0]["ground"]["dark"], "#1f1f22");
        assert_eq!(json[4]["dark"]["accent"], "#88c0d0");
    }
}
