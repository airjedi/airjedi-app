use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use catppuccin::FlavorName;
use egui_aesthetix::Aesthetix;

/// Returns the egui FontFamily used for Phosphor icon glyphs.
pub fn icon_font_family() -> egui::FontFamily {
    egui::FontFamily::Name("phosphor_icons".into())
}

/// Returns a FontId for rendering Phosphor icons at the given size.
/// Falls back to Proportional if the icon font isn't loaded yet.
pub fn icon_font_id(size: f32, ctx: &egui::Context) -> egui::FontId {
    let family = icon_font_family();
    let available = ctx.fonts(|f| f.families().iter().any(|f2| f2 == &family));
    if available {
        egui::FontId::new(size, family)
    } else {
        egui::FontId::new(size, egui::FontFamily::Proportional)
    }
}

// ── Conversion helpers ──────────────────────────────────────────────

/// Convert an `egui::Color32` to a `bevy::color::Color`.
fn color32_to_bevy(c: egui::Color32) -> Color {
    Color::srgba(
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
        c.a() as f32 / 255.0,
    )
}

/// Convert a `bevy::color::Color` to `egui::Color32`.
pub fn to_egui_color32(color: Color) -> egui::Color32 {
    let srgba = color.to_srgba();
    egui::Color32::from_rgba_unmultiplied(
        (srgba.red * 255.0) as u8,
        (srgba.green * 255.0) as u8,
        (srgba.blue * 255.0) as u8,
        (srgba.alpha * 255.0) as u8,
    )
}

/// Convert a `bevy::color::Color` to `egui::Color32` with a custom alpha.
pub fn to_egui_color32_alpha(color: Color, alpha: u8) -> egui::Color32 {
    let srgba = color.to_srgba();
    egui::Color32::from_rgba_unmultiplied(
        (srgba.red * 255.0) as u8,
        (srgba.green * 255.0) as u8,
        (srgba.blue * 255.0) as u8,
        alpha,
    )
}

/// Blend two Color32 values by a ratio (0.0 = all `a`, 1.0 = all `b`).
fn blend_color32(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let inv = 1.0 - t;
    egui::Color32::from_rgb(
        (a.r() as f32 * inv + b.r() as f32 * t) as u8,
        (a.g() as f32 * inv + b.g() as f32 * t) as u8,
        (a.b() as f32 * inv + b.b() as f32 * t) as u8,
    )
}

// ── AppTheme resource ───────────────────────────────────────────────

/// Central theme resource for the application.
///
/// Wraps an egui-aesthetix theme and provides semantic color accessors
/// returning `bevy::color::Color` values.
#[derive(Resource)]
pub struct AppTheme {
    inner: Box<dyn Aesthetix + Send + Sync>,
    name: String,
    // Extended colors not covered by Aesthetix
    ext_text_dim: egui::Color32,
    ext_bg_overlay: egui::Color32,
    ext_altitude_low: egui::Color32,
    ext_altitude_high: egui::Color32,
    ext_altitude_ultra: egui::Color32,
}

impl AppTheme {
    /// Create a new AppTheme from an Aesthetix implementation with default extended colors.
    pub fn new(name: impl Into<String>, theme: impl Aesthetix + Send + Sync + 'static) -> Self {
        let text = theme
            .fg_primary_text_color_visuals()
            .unwrap_or(egui::Color32::WHITE);
        let bg = theme.bg_primary_color_visuals();
        let ext_text_dim = blend_color32(text, bg, 0.4);
        let ext_bg_overlay = theme.bg_auxiliary_color_visuals();
        let ext_altitude_low = theme.secondary_accent_color_visuals();
        let ext_altitude_high = blend_color32(
            theme.fg_warn_text_color_visuals(),
            theme.fg_error_text_color_visuals(),
            0.5,
        );
        let ext_altitude_ultra = theme.secondary_accent_color_visuals();

        Self {
            inner: Box::new(theme),
            name: name.into(),
            ext_text_dim,
            ext_bg_overlay,
            ext_altitude_low,
            ext_altitude_high,
            ext_altitude_ultra,
        }
    }

    /// Create with explicit extended colors (used by Catppuccin adapters).
    pub fn with_extended_colors(
        mut self,
        text_dim: egui::Color32,
        bg_overlay: egui::Color32,
        altitude_low: egui::Color32,
        altitude_high: egui::Color32,
        altitude_ultra: egui::Color32,
    ) -> Self {
        self.ext_text_dim = text_dim;
        self.ext_bg_overlay = bg_overlay;
        self.ext_altitude_low = altitude_low;
        self.ext_altitude_high = altitude_high;
        self.ext_altitude_ultra = altitude_ultra;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the egui Style from the underlying Aesthetix theme.
    pub fn egui_style(&self) -> egui::Style {
        self.inner.custom_style()
    }

    // ── Background colors ──

    pub fn bg_primary(&self) -> Color {
        color32_to_bevy(self.inner.bg_primary_color_visuals())
    }

    pub fn bg_secondary(&self) -> Color {
        color32_to_bevy(self.inner.bg_secondary_color_visuals())
    }

    pub fn bg_triage(&self) -> Color {
        color32_to_bevy(self.inner.bg_triage_color_visuals())
    }

    pub fn bg_auxiliary(&self) -> Color {
        color32_to_bevy(self.inner.bg_auxiliary_color_visuals())
    }

    pub fn bg_contrast(&self) -> Color {
        color32_to_bevy(self.inner.bg_contrast_color_visuals())
    }

    pub fn bg_overlay(&self) -> Color {
        color32_to_bevy(self.ext_bg_overlay)
    }

    // ── Accent colors ──

    pub fn accent_primary(&self) -> Color {
        color32_to_bevy(self.inner.primary_accent_color_visuals())
    }

    pub fn accent_secondary(&self) -> Color {
        color32_to_bevy(self.inner.secondary_accent_color_visuals())
    }

    // ── Text colors ──

    pub fn text_primary(&self) -> Color {
        color32_to_bevy(
            self.inner
                .fg_primary_text_color_visuals()
                .unwrap_or(egui::Color32::WHITE),
        )
    }

    pub fn text_dim(&self) -> Color {
        color32_to_bevy(self.ext_text_dim)
    }

    pub fn text_success(&self) -> Color {
        color32_to_bevy(self.inner.fg_success_text_color_visuals())
    }

    pub fn text_warn(&self) -> Color {
        color32_to_bevy(self.inner.fg_warn_text_color_visuals())
    }

    pub fn text_error(&self) -> Color {
        color32_to_bevy(self.inner.fg_error_text_color_visuals())
    }

    // ── Domain-specific colors ──

    pub fn altitude_low(&self) -> Color {
        color32_to_bevy(self.ext_altitude_low)
    }

    pub fn altitude_high(&self) -> Color {
        color32_to_bevy(self.ext_altitude_high)
    }

    pub fn altitude_ultra(&self) -> Color {
        color32_to_bevy(self.ext_altitude_ultra)
    }
}

// ── WidgetTheme ─────────────────────────────────────────────────────

/// Lightweight theme colors for custom widgets.
///
/// Extracted from `AppTheme` so widgets don't depend on the full
/// Aesthetix trait or Bevy ECS resources.
pub struct WidgetTheme {
    pub bg_primary: egui::Color32,
    pub bg_secondary: egui::Color32,
    pub accent: egui::Color32,
    pub border: egui::Color32,
    pub text: egui::Color32,
    pub text_dim: egui::Color32,
    pub shadow_color: egui::Color32,
}

impl From<&AppTheme> for WidgetTheme {
    fn from(theme: &AppTheme) -> Self {
        Self {
            bg_primary: theme.inner.bg_primary_color_visuals(),
            bg_secondary: theme.inner.bg_secondary_color_visuals(),
            accent: theme.inner.primary_accent_color_visuals(),
            border: theme.inner.bg_contrast_color_visuals(),
            text: theme
                .inner
                .fg_primary_text_color_visuals()
                .unwrap_or(egui::Color32::WHITE),
            text_dim: theme.ext_text_dim,
            shadow_color: egui::Color32::from_black_alpha(50),
        }
    }
}

impl Default for AppTheme {
    fn default() -> Self {
        catppuccin_mocha()
    }
}

// ── Catppuccin adapter themes ───────────────────────────────────────
//
// Each adapter implements the Aesthetix trait using catppuccin palette colors.
// Extended colors (text_dim, bg_overlay, altitude colors) use exact palette values.

/// Helper: get the catppuccin Color32 for a given flavor and color accessor.
fn cat_color(
    flavor: FlavorName,
    f: fn(&catppuccin::FlavorColors) -> &catppuccin::Color,
) -> egui::Color32 {
    let c = f(&catppuccin::PALETTE.get_flavor(flavor).colors);
    egui::Color32::from_rgb(c.rgb.r, c.rgb.g, c.rgb.b)
}

macro_rules! catppuccin_theme {
    ($struct_name:ident, $ctor_fn:ident, $flavor:expr, $display_name:expr, $is_dark:expr) => {
        pub struct $struct_name;

        impl Aesthetix for $struct_name {
            fn name(&self) -> &str {
                $display_name
            }
            fn primary_accent_color_visuals(&self) -> egui::Color32 {
                cat_color($flavor, |c| &c.blue)
            }
            fn secondary_accent_color_visuals(&self) -> egui::Color32 {
                cat_color($flavor, |c| &c.mauve)
            }
            fn bg_primary_color_visuals(&self) -> egui::Color32 {
                cat_color($flavor, |c| &c.base)
            }
            fn bg_secondary_color_visuals(&self) -> egui::Color32 {
                cat_color($flavor, |c| &c.mantle)
            }
            fn bg_triage_color_visuals(&self) -> egui::Color32 {
                cat_color($flavor, |c| &c.crust)
            }
            fn bg_auxiliary_color_visuals(&self) -> egui::Color32 {
                cat_color($flavor, |c| &c.surface0)
            }
            fn bg_contrast_color_visuals(&self) -> egui::Color32 {
                cat_color($flavor, |c| &c.surface1)
            }
            fn fg_primary_text_color_visuals(&self) -> Option<egui::Color32> {
                Some(cat_color($flavor, |c| &c.text))
            }
            fn fg_success_text_color_visuals(&self) -> egui::Color32 {
                cat_color($flavor, |c| &c.green)
            }
            fn fg_warn_text_color_visuals(&self) -> egui::Color32 {
                cat_color($flavor, |c| &c.yellow)
            }
            fn fg_error_text_color_visuals(&self) -> egui::Color32 {
                cat_color($flavor, |c| &c.red)
            }
            fn dark_mode_visuals(&self) -> bool {
                $is_dark
            }
            fn margin_style(&self) -> i8 {
                10
            }
            fn button_padding(&self) -> egui::Vec2 {
                egui::Vec2::new(8.0, 4.0)
            }
            fn item_spacing_style(&self) -> f32 {
                8.0
            }
            fn scroll_bar_width_style(&self) -> f32 {
                12.0
            }
            fn rounding_visuals(&self) -> u8 {
                6
            }
            fn custom_text_styles(
                &self,
            ) -> std::collections::BTreeMap<egui::TextStyle, egui::FontId> {
                use egui::FontFamily::{Monospace, Proportional};
                [
                    (
                        egui::TextStyle::Small,
                        egui::FontId::new(10.0, Proportional),
                    ),
                    (egui::TextStyle::Body, egui::FontId::new(13.0, Proportional)),
                    (
                        egui::TextStyle::Button,
                        egui::FontId::new(12.0, Proportional),
                    ),
                    (
                        egui::TextStyle::Heading,
                        egui::FontId::new(15.0, Proportional),
                    ),
                    (
                        egui::TextStyle::Monospace,
                        egui::FontId::new(12.0, Monospace),
                    ),
                ]
                .into()
            }
        }

        pub fn $ctor_fn() -> AppTheme {
            let flavor = $flavor;
            AppTheme::new($display_name, $struct_name).with_extended_colors(
                cat_color(flavor, |c| &c.subtext0),
                cat_color(flavor, |c| &c.overlay0),
                cat_color(flavor, |c| &c.teal),
                cat_color(flavor, |c| &c.peach),
                cat_color(flavor, |c| &c.mauve),
            )
        }
    };
}

catppuccin_theme!(
    CatppuccinMochaTheme,
    catppuccin_mocha,
    FlavorName::Mocha,
    "Catppuccin Mocha",
    true
);
catppuccin_theme!(
    CatppuccinMacchiatoTheme,
    catppuccin_macchiato,
    FlavorName::Macchiato,
    "Catppuccin Macchiato",
    true
);
catppuccin_theme!(
    CatppuccinFrappeTheme,
    catppuccin_frappe,
    FlavorName::Frappe,
    "Catppuccin Frappe",
    true
);
catppuccin_theme!(
    CatppuccinLatteTheme,
    catppuccin_latte,
    FlavorName::Latte,
    "Catppuccin Latte",
    false
);

// ── Cockpit Dark theme ──────────────────────────────────────────────
//
// Standalone theme based on the Catppuccin Mocha palette.
// All color values are hardcoded — no dependency on the catppuccin crate.

pub struct CockpitDarkTheme;

impl Aesthetix for CockpitDarkTheme {
    fn name(&self) -> &str {
        "Cockpit Dark"
    }
    fn primary_accent_color_visuals(&self) -> egui::Color32 {
        // burned orange
        egui::Color32::from_rgb(204, 102, 34)
    }
    fn secondary_accent_color_visuals(&self) -> egui::Color32 {
        // warm amber
        egui::Color32::from_rgb(224, 148, 64)
    }
    fn bg_primary_color_visuals(&self) -> egui::Color32 {
        // charcoal base
        egui::Color32::from_rgb(39, 42, 46)
    }
    fn bg_secondary_color_visuals(&self) -> egui::Color32 {
        // deep charcoal
        egui::Color32::from_rgb(28, 30, 34)
    }
    fn bg_triage_color_visuals(&self) -> egui::Color32 {
        // darkest charcoal
        egui::Color32::from_rgb(20, 21, 24)
    }
    fn bg_auxiliary_color_visuals(&self) -> egui::Color32 {
        // medium charcoal
        egui::Color32::from_rgb(50, 53, 58)
    }
    fn bg_contrast_color_visuals(&self) -> egui::Color32 {
        // light charcoal
        egui::Color32::from_rgb(62, 65, 71)
    }
    fn fg_primary_text_color_visuals(&self) -> Option<egui::Color32> {
        // light gray
        Some(egui::Color32::from_rgb(208, 210, 214))
    }
    fn fg_success_text_color_visuals(&self) -> egui::Color32 {
        // muted green
        egui::Color32::from_rgb(125, 186, 106)
    }
    fn fg_warn_text_color_visuals(&self) -> egui::Color32 {
        // amber gold
        egui::Color32::from_rgb(224, 176, 80)
    }
    fn fg_error_text_color_visuals(&self) -> egui::Color32 {
        // muted red
        egui::Color32::from_rgb(212, 80, 80)
    }
    fn dark_mode_visuals(&self) -> bool {
        true
    }
    fn margin_style(&self) -> i8 {
        10
    }
    fn button_padding(&self) -> egui::Vec2 {
        egui::Vec2::new(8.0, 4.0)
    }
    fn item_spacing_style(&self) -> f32 {
        8.0
    }
    fn scroll_bar_width_style(&self) -> f32 {
        12.0
    }
    fn rounding_visuals(&self) -> u8 {
        6
    }
    fn custom_text_styles(&self) -> std::collections::BTreeMap<egui::TextStyle, egui::FontId> {
        use egui::FontFamily::{Monospace, Proportional};
        [
            (
                egui::TextStyle::Small,
                egui::FontId::new(10.0, Proportional),
            ),
            (egui::TextStyle::Body, egui::FontId::new(13.0, Proportional)),
            (
                egui::TextStyle::Button,
                egui::FontId::new(12.0, Proportional),
            ),
            (
                egui::TextStyle::Heading,
                egui::FontId::new(15.0, Proportional),
            ),
            (
                egui::TextStyle::Monospace,
                egui::FontId::new(12.0, Monospace),
            ),
        ]
        .into()
    }
    fn widget_hovered_visual(&self) -> egui::style::WidgetVisuals {
        let rounding = egui::CornerRadius::same(self.rounding_visuals());
        egui::style::WidgetVisuals {
            bg_fill: self.bg_auxiliary_color_visuals(),
            weak_bg_fill: self.bg_auxiliary_color_visuals(),
            bg_stroke: egui::Stroke {
                width: 1.0,
                color: self.primary_accent_color_visuals(),
            },
            fg_stroke: egui::Stroke {
                width: 1.5,
                color: self.fg_primary_text_color_visuals().unwrap_or_default(),
            },
            corner_radius: rounding,
            expansion: 2.0,
        }
    }
    fn custom_active_widget_visual(&self) -> egui::style::WidgetVisuals {
        let rounding = egui::CornerRadius::same(self.rounding_visuals());
        egui::style::WidgetVisuals {
            bg_fill: self.bg_contrast_color_visuals(),
            weak_bg_fill: self.bg_contrast_color_visuals(),
            bg_stroke: egui::Stroke {
                width: 1.0,
                color: self.primary_accent_color_visuals(),
            },
            fg_stroke: egui::Stroke {
                width: 2.0,
                color: self.fg_primary_text_color_visuals().unwrap_or_default(),
            },
            corner_radius: rounding,
            expansion: 1.0,
        }
    }
    fn custom_open_widget_visual(&self) -> egui::style::WidgetVisuals {
        let rounding = egui::CornerRadius::same(self.rounding_visuals());
        egui::style::WidgetVisuals {
            bg_fill: self.bg_auxiliary_color_visuals(),
            weak_bg_fill: self.bg_auxiliary_color_visuals(),
            bg_stroke: egui::Stroke {
                width: 1.0,
                color: self.primary_accent_color_visuals(),
            },
            fg_stroke: egui::Stroke {
                width: 1.0,
                color: self.fg_primary_text_color_visuals().unwrap_or_default(),
            },
            corner_radius: rounding,
            expansion: 0.0,
        }
    }
}

pub fn cockpit_dark() -> AppTheme {
    AppTheme::new("Cockpit Dark", CockpitDarkTheme).with_extended_colors(
        // dim gray text
        egui::Color32::from_rgb(128, 133, 144),
        // overlay gray
        egui::Color32::from_rgb(74, 77, 84),
        // muted teal (altitude low)
        egui::Color32::from_rgb(90, 158, 160),
        // burned orange (altitude high)
        egui::Color32::from_rgb(204, 102, 34),
        // warm amber (altitude ultra)
        egui::Color32::from_rgb(224, 148, 64),
    )
}

// ── Theme registry ──────────────────────────────────────────────────

pub type ThemeConstructor = fn() -> AppTheme;

#[derive(Resource)]
pub struct ThemeRegistry {
    themes: Vec<(String, ThemeConstructor)>,
}

impl ThemeRegistry {
    pub fn new() -> Self {
        let mut reg = Self { themes: Vec::new() };

        // Custom themes
        reg.register("Cockpit Dark", cockpit_dark);

        // Catppuccin themes
        reg.register("Catppuccin Mocha", catppuccin_mocha);
        reg.register("Catppuccin Macchiato", catppuccin_macchiato);
        reg.register("Catppuccin Frappe", catppuccin_frappe);
        reg.register("Catppuccin Latte", catppuccin_latte);

        // egui-aesthetix built-in themes
        reg.register("Standard Dark", || {
            AppTheme::new("Standard Dark", egui_aesthetix::themes::StandardDark)
        });
        reg.register("Standard Light", || {
            AppTheme::new("Standard Light", egui_aesthetix::themes::StandardLight)
        });
        reg.register("Carl Dark", || {
            AppTheme::new("Carl Dark", egui_aesthetix::themes::CarlDark)
        });
        reg.register("Nord Dark", || {
            AppTheme::new("Nord Dark", egui_aesthetix::themes::NordDark)
        });
        reg.register("Nord Light", || {
            AppTheme::new("Nord Light", egui_aesthetix::themes::NordLight)
        });
        reg.register("Tokyo Night Storm", || {
            AppTheme::new("Tokyo Night Storm", egui_aesthetix::themes::TokyoNightStorm)
        });

        reg
    }

    pub fn register(&mut self, name: &str, constructor: ThemeConstructor) {
        self.themes.push((name.to_string(), constructor));
    }

    pub fn get(&self, name: &str) -> Option<AppTheme> {
        self.themes
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, ctor)| ctor())
    }

    pub fn names(&self) -> Vec<&str> {
        self.themes.iter().map(|(n, _)| n.as_str()).collect()
    }
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── egui theme application system ───────────────────────────────────

/// System that applies the active theme's egui Style whenever `AppTheme` changes.
/// Also loads the Phosphor icon font on first run.
pub fn apply_egui_theme(
    theme: Res<AppTheme>,
    mut contexts: EguiContexts,
    mut fonts_loaded: Local<bool>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    if !*fonts_loaded {
        let mut fonts = egui::FontDefinitions::default();

        // Use Inter for crisp UI text at small sizes
        let inter_data = include_bytes!("../assets/fonts/Inter-Regular.ttf");
        fonts.font_data.insert(
            "Inter".to_owned(),
            egui::FontData::from_static(inter_data).into(),
        );
        fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .insert(0, "Inter".to_owned());

        // Load Phosphor icon font into a dedicated family so it doesn't
        // conflict with Inter's PUA mappings. Use icon_font_family()
        // when rendering icon text.
        fonts.font_data.insert(
            "phosphor".to_owned(),
            egui_phosphor::Variant::Regular.font_data().into(),
        );
        fonts.families.insert(
            icon_font_family(),
            vec!["phosphor".to_owned(), "Inter".to_owned()],
        );

        ctx.set_fonts(fonts);
        *fonts_loaded = true;
    }
    if theme.is_changed() {
        let mut style = theme.egui_style();
        // Remove default margins so panels sit flush against each other
        style.spacing.window_margin = egui::Margin::ZERO;
        style.visuals.clip_rect_margin = 0.0;
        ctx.set_style(style);
    }
}
