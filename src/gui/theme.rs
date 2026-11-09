//! How the window looks: Noctorium's Night theme, the system's own typeface, and the few pieces drawn the
//! same way everywhere -- cards, buttons, the switcher.
//!
//! Night, as Noctorium Stats has it: this window sits beside the player, which most people keep on Night.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Rect,
    Response, RichText, Sense, Stroke, StrokeKind, TextStyle, Ui, Vec2,
};
use std::sync::Arc;

pub const PAGE: Color32 = Color32::from_rgb(0x00, 0x00, 0x00);
pub const PANEL: Color32 = Color32::from_rgb(0x07, 0x05, 0x0A);
pub const CARD: Color32 = Color32::from_rgb(0x15, 0x10, 0x1C);
/// A field or a track inside a card: darker than the card, so it reads as a hollow in it.
pub const WELL: Color32 = Color32::from_rgb(0x0C, 0x09, 0x10);
/// Hairlines: card edges, grid lines, the empty part of a bar.
pub const LINE: Color32 = Color32::from_rgb(0x2A, 0x22, 0x35);
pub const TEXT: Color32 = Color32::from_rgb(0xF8, 0xF4, 0xFF);
pub const SUBTEXT: Color32 = Color32::from_rgb(0xCF, 0xC5, 0xDA);
/// Third in line after the subtext: axis labels, ranks, the small print.
pub const FAINT: Color32 = Color32::from_rgb(0x8E, 0x84, 0x9C);
pub const ACCENT: Color32 = Color32::from_rgb(0xB4, 0x7C, 0xFF);
/// The accent at its brightest, for the one bar that is the most of its chart.
pub const ACCENT_BRIGHT: Color32 = Color32::from_rgb(0xD4, 0xB4, 0xFF);
/// Writing on the accent. The accent is a pale violet, so what sits on it has to be dark.
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x1A, 0x10, 0x28);
pub const ERROR: Color32 = Color32::from_rgb(0xFF, 0x7A, 0x8A);
pub const WARNING: Color32 = Color32::from_rgb(0xFF, 0xC4, 0x6B);

/// The accent at a fraction of its strength, for washes behind things.
pub fn accent_wash(alpha: f32) -> Color32 {
    ACCENT.gamma_multiply(alpha)
}

pub fn rgb(colour: [u8; 3]) -> Color32 {
    Color32::from_rgb(colour[0], colour[1], colour[2])
}

/// The heavier faces, by name; Proportional is the regular one.
pub const SEMIBOLD: &str = "semibold";
pub const BOLD: &str = "bold";

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

pub const RADIUS: u8 = 16;
pub const GAP: f32 = 14.0;

/// Fonts, colours and spacing, once, before the first frame.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.options_mut(|o| o.theme_preference = egui::ThemePreference::Dark);
    ctx.style_mut_of(egui::Theme::Dark, style);
}

fn style(style: &mut egui::Style) {
    use TextStyle::*;
    style.text_styles = [
        (Heading, bold(24.0)),
        (Body, regular(14.0)),
        (Monospace, FontId::new(13.0, FontFamily::Monospace)),
        (Button, regular(14.0)),
        (Small, regular(12.0)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size.y = 34.0;
    style.spacing.scroll = egui::style::ScrollStyle::thin();
    style.interaction.selectable_labels = false;

    let v = &mut style.visuals;
    v.dark_mode = true;
    v.override_text_color = None;
    v.panel_fill = PAGE;
    v.window_fill = CARD;
    v.window_stroke = stroke(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = WELL;
    v.text_edit_bg_color = Some(WELL);
    v.faint_bg_color = PANEL;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.45);
    v.selection.stroke = stroke(1.5, ACCENT);
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(160),
    };
    v.window_shadow = v.popup_shadow;

    let shape = CornerRadius::same(10);
    for (widgets, fill, edge, text) in [
        (&mut v.widgets.noninteractive, CARD, LINE, SUBTEXT),
        (
            &mut v.widgets.inactive,
            Color32::from_rgb(0x22, 0x1A, 0x2C),
            LINE,
            TEXT,
        ),
        (
            &mut v.widgets.hovered,
            Color32::from_rgb(0x2C, 0x22, 0x39),
            Color32::from_rgb(0x4A, 0x3A, 0x5E),
            TEXT,
        ),
        (
            &mut v.widgets.active,
            Color32::from_rgb(0x36, 0x29, 0x47),
            ACCENT,
            TEXT,
        ),
        (
            &mut v.widgets.open,
            Color32::from_rgb(0x2C, 0x22, 0x39),
            LINE,
            TEXT,
        ),
    ] {
        widgets.bg_fill = fill;
        widgets.weak_bg_fill = fill;
        widgets.bg_stroke = stroke(1.0, edge);
        widgets.fg_stroke = stroke(1.5, text);
        widgets.corner_radius = shape;
        widgets.expansion = 0.0;
    }
}

/// The system's own typeface where there is one this can find -- Segoe UI on Windows, Helvetica Neue on a
/// Mac, Noto Sans or DejaVu Sans on Linux -- with egui's own fonts behind it for anything it lacks, the
/// symbols above all. Read from the system rather than carried, so the window looks like its neighbours and
/// the program carries no font of its own.
fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let fallback: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    let faces = system_faces();
    let mut family = |name: &str, face: Option<&(Vec<u8>, u32)>| {
        let mut chain = Vec::new();
        if let Some((bytes, index)) = face {
            let mut data = FontData::from_owned(bytes.clone());
            data.index = *index;
            fonts.font_data.insert(name.to_string(), Arc::new(data));
            chain.push(name.to_string());
        }
        chain.extend(fallback.iter().cloned());
        let key = if name == "regular" {
            FontFamily::Proportional
        } else {
            FontFamily::Name(name.into())
        };
        fonts.families.insert(key, chain);
    };
    family("regular", faces.as_ref().map(|f| &f.0));
    family(SEMIBOLD, faces.as_ref().map(|f| &f.1));
    family(BOLD, faces.as_ref().map(|f| &f.2));
    fonts
}

type Face = (Vec<u8>, u32);

/// Regular, semibold and bold, as files and the face within each.
fn system_faces() -> Option<(Face, Face, Face)> {
    let read = |path: &str| std::fs::read(path).ok().map(|bytes| (bytes, 0u32));

    if cfg!(windows) {
        let fonts = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into()) + r"\Fonts\";
        let regular = read(&(fonts.clone() + "segoeui.ttf"))?;
        let bold = read(&(fonts.clone() + "segoeuib.ttf"))?;
        let semibold = read(&(fonts + "seguisb.ttf")).unwrap_or_else(|| bold.clone());
        return Some((regular, semibold, bold));
    }

    if cfg!(target_os = "macos") {
        // A collection, whose faces are in no documented order, so each is found by its weight.
        for path in [
            "/System/Library/Fonts/HelveticaNeue.ttc",
            "/System/Library/Fonts/Helvetica.ttc",
        ] {
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            let pick = |weights: &[u16]| {
                weights.iter().find_map(|&weight| {
                    face_of_weight(&bytes, weight).map(|index| (bytes.clone(), index))
                })
            };
            if let (Some(r), Some(b)) = (pick(&[400]), pick(&[700])) {
                let s = pick(&[600, 500]).unwrap_or_else(|| b.clone());
                return Some((r, s, b));
            }
        }
        return None;
    }

    // Linux: the faces most distributions ship, wherever each puts them.
    for (regular, semibold, bold) in [
        (
            "NotoSans-Regular.ttf",
            "NotoSans-SemiBold.ttf",
            "NotoSans-Bold.ttf",
        ),
        (
            "DejaVuSans.ttf",
            "DejaVuSans-Bold.ttf",
            "DejaVuSans-Bold.ttf",
        ),
        (
            "LiberationSans-Regular.ttf",
            "LiberationSans-Bold.ttf",
            "LiberationSans-Bold.ttf",
        ),
    ] {
        for folder in [
            "/usr/share/fonts/truetype/noto/",
            "/usr/share/fonts/noto/",
            "/usr/share/fonts/google-noto/",
            "/usr/share/fonts/google-noto-vf/",
            "/usr/share/fonts/truetype/dejavu/",
            "/usr/share/fonts/dejavu/",
            "/usr/share/fonts/dejavu-sans-fonts/",
            "/usr/share/fonts/TTF/",
            "/usr/share/fonts/truetype/liberation/",
            "/usr/share/fonts/liberation-sans/",
        ] {
            if let (Some(r), Some(b)) = (
                read(&format!("{folder}{regular}")),
                read(&format!("{folder}{bold}")),
            ) {
                let s = read(&format!("{folder}{semibold}")).unwrap_or_else(|| b.clone());
                return Some((r, s, b));
            }
        }
    }
    None
}

/// The upright, normal-width face of [weight] in a font collection.
fn face_of_weight(bytes: &[u8], weight: u16) -> Option<u32> {
    let count = ttf_parser::fonts_in_collection(bytes).unwrap_or(1);
    (0..count).find(|&index| {
        ttf_parser::Face::parse(bytes, index).is_ok_and(|face| {
            face.weight().to_number() == weight
                && !face.is_italic()
                && face.width() == ttf_parser::Width::Normal
        })
    })
}

// ------------------------------------------------------------------------------------------- the pieces

/// A card: the rounded panel everything on the dashboard sits in.
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(stroke(1.0, Color32::from_rgb(0x1F, 0x18, 0x29)))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::same(20))
}

/// A card's title, and an optional line of detail on the right of it.
pub fn card_title(ui: &mut Ui, title: &str, detail: Option<&str>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(semibold(16.0)).color(TEXT));
        if let Some(detail) = detail {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(RichText::new(detail).font(regular(12.5)).color(FAINT))
                        .truncate(),
                );
            });
        }
    });
    ui.add_space(10.0);
}

/// The button that does the thing: violet, with dark writing.
pub fn primary(ui: &mut Ui, text: &str, width: f32, enabled: bool) -> Response {
    // Grown on both sides, so the label sits in the middle of however wide the button is made.
    let label = RichText::new(text).font(semibold(15.0)).color(ON_ACCENT);
    let button = egui::Button::new((egui::Atom::grow(), label, egui::Atom::grow()))
        .fill(if enabled {
            ACCENT
        } else {
            ACCENT.gamma_multiply(0.5)
        })
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(12))
        .min_size(egui::vec2(width, 42.0));
    ui.add_enabled(enabled, button)
}

/// A quieter button, for everything else.
pub fn secondary(ui: &mut Ui, text: impl Into<String>) -> Response {
    let button = egui::Button::new(RichText::new(text.into()).font(regular(14.0)).color(TEXT))
        .fill(Color32::from_rgb(0x1C, 0x15, 0x25))
        .stroke(stroke(1.0, LINE))
        .corner_radius(CornerRadius::same(10))
        .min_size(egui::vec2(0.0, 34.0));
    ui.add(button)
}

/// A link, in the accent, that opens in the browser.
pub fn link(ui: &mut Ui, text: &str, url: &str) -> Response {
    let response = ui.add(egui::Link::new(RichText::new(text).color(ACCENT)));
    if response.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
    response.on_hover_text(url)
}

/// A row of options of which one is chosen, drawn as one pill with the chosen part lit. Returns the option
/// clicked, if one was.
pub fn segmented(
    ui: &mut Ui,
    options: &[&str],
    chosen: usize,
    segment_width: f32,
) -> Option<usize> {
    let height = 36.0;
    let pad = 4.0;
    let size = egui::vec2(segment_width * options.len() as f32 + pad * 2.0, height);
    let (outer, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(outer.expand(1.0));
    painter.rect(
        outer,
        CornerRadius::same(12),
        PANEL,
        stroke(1.0, LINE),
        StrokeKind::Inside,
    );

    let mut clicked = None;
    for (index, label) in options.iter().enumerate() {
        let rect = Rect::from_min_size(
            egui::pos2(
                outer.left() + pad + segment_width * index as f32,
                outer.top() + pad,
            ),
            egui::vec2(segment_width, height - pad * 2.0),
        );
        let response = ui.interact(rect, ui.id().with(("segment", index)), Sense::click());
        let selected = index == chosen;
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, *label)
        });
        let (fill, colour) = if selected {
            (ACCENT, ON_ACCENT)
        } else if response.hovered() {
            (Color32::from_rgb(0x1E, 0x17, 0x28), TEXT)
        } else {
            (Color32::TRANSPARENT, SUBTEXT)
        };
        painter.rect_filled(rect, CornerRadius::same(9), fill);
        let font = if selected {
            semibold(13.5)
        } else {
            regular(13.5)
        };
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            font,
            colour,
        );
        if response.clicked() && !selected {
            clicked = Some(index);
        }
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    clicked
}

/// A dot in a service's colour.
pub fn dot(ui: &mut Ui, colour: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), size * 0.32, colour);
}

/// A soft violet glow, as Noctorium's own pages have behind their headings: a disc of the accent fading to
/// nothing, drawn as a fan of triangles with the colour at the centre and none at the rim.
pub fn glow(painter: &egui::Painter, centre: egui::Pos2, radius: Vec2, strength: f32) {
    let mut mesh = egui::Mesh::default();
    let rings = 16;
    let steps = 64;
    mesh.colored_vertex(centre, ACCENT.gamma_multiply(strength));
    for ring in 1..=rings {
        let t = ring as f32 / rings as f32;
        // Falling off smoothly rather than in a straight line, so there is no visible edge.
        let alpha = strength * (1.0 - t).powf(2.2);
        for step in 0..steps {
            let angle = step as f32 / steps as f32 * std::f32::consts::TAU;
            let at = centre + egui::vec2(angle.cos() * radius.x * t, angle.sin() * radius.y * t);
            mesh.colored_vertex(at, ACCENT.gamma_multiply(alpha));
        }
    }
    for step in 0..steps {
        let next = (step + 1) % steps;
        mesh.add_triangle(0, 1 + step, 1 + next);
    }
    for ring in 1..rings {
        let inner = 1 + (ring - 1) * steps;
        let outer = 1 + ring * steps;
        for step in 0..steps {
            let next = (step + 1) % steps;
            mesh.add_triangle(inner + step, outer + step, outer + next);
            mesh.add_triangle(inner + step, outer + next, inner + next);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// A stroke, its width typed: egui takes anything that converts to a float, which leaves a bare `1.0` for
/// the compiler to guess at.
pub fn stroke(width: f32, colour: Color32) -> Stroke {
    Stroke::new(width, colour)
}
