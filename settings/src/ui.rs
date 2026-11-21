//! 设置界面的外观：配色、卡片、设置行、开关、步进器、下拉框和左侧导航。
//!
//! 控件全用画笔自己画，尺寸和颜色集中在这里改；页面代码只关心「哪一行放什么」。

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Margin, Painter, Rect, Response, RichText, Sense,
    Stroke, StrokeKind, Ui, Vec2,
};

/// 品牌绿，和 fcitx5 主题里的高亮色一致。
pub const ACCENT: Color32 = Color32::from_rgb(0x0E, 0xAF, 0x76);
/// 侧栏底色，比内容区多一点绿。
pub const SIDEBAR: Color32 = Color32::from_rgb(0xF6, 0xFB, 0xF8);
/// 内容区底色。
pub const WINDOW: Color32 = Color32::from_rgb(0xFA, 0xFC, 0xFB);
/// 卡片底色。
pub const CARD: Color32 = Color32::WHITE;
/// 分隔线、卡片边框。
pub const LINE: Color32 = Color32::from_rgb(0xEA, 0xEF, 0xEC);
/// 正文。
pub const TEXT: Color32 = Color32::from_rgb(0x1D, 0x23, 0x21);
/// 说明、次要文字。
pub const MUTED: Color32 = Color32::from_rgb(0x7A, 0x85, 0x80);
/// 开关关掉时的槽底色。
pub const TRACK_OFF: Color32 = Color32::from_rgb(0xD8, 0xDE, 0xDB);
/// 圆形小按钮的底色。
pub const BUTTON_BG: Color32 = Color32::from_rgb(0xEE, 0xF2, 0xF0);
/// 下拉框、输入框的边框。
pub const BORDER: Color32 = Color32::from_rgb(0xDD, 0xE4, 0xE1);

/// 左侧导航的图标（用画笔描，不依赖字体里有没有符号）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// 常规：四角星。
    General,
    /// 输入：键盘。
    Input,
    /// 语音：话筒。
    Voice,
    /// 增强：笑脸。
    Emoji,
    /// 自定义词：小书本。
    Book,
}

/// 设置界面的字体、控件配色和间距，启动时调一次。
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Light);
    ctx.style_mut_of(egui::Theme::Light, |style| {
        style.visuals = egui::Visuals::light();
        style.visuals.panel_fill = WINDOW;
        style.visuals.window_fill = WINDOW;
        style.visuals.extreme_bg_color = Color32::from_rgb(0xF4, 0xF7, 0xF5);
        style.visuals.selection.bg_fill = ACCENT;
        style.visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        style.visuals.hyperlink_color = ACCENT;
        style.visuals.popup_shadow = egui::epaint::Shadow::NONE;
        style.visuals.window_shadow = egui::epaint::Shadow::NONE;
        style.visuals.window_stroke = Stroke::new(1.0, BORDER);
        style.visuals.window_corner_radius = CornerRadius::same(10);
        style.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        style.visuals.widgets.inactive.bg_stroke = Stroke::NONE;
        style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xEF, 0xF5, 0xF2);
        style.visuals.widgets.hovered.bg_stroke = Stroke::NONE;
        style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(0xE4, 0xEF, 0xEA);
        style.visuals.widgets.active.bg_stroke = Stroke::NONE;
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;
    });
}

/// 页面标题（右边内容区最上面那行大字）。
pub fn title(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(
        RichText::new(text)
            .size(23.0)
            .strong()
            .color(Color32::from_rgb(0x0F, 0x15, 0x13)),
    );
    ui.add_space(6.0);
}

/// 分组小标题（「候选词」「外观」这种）。
pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(18.0);
    ui.label(
        RichText::new(text)
            .size(13.0)
            .strong()
            .color(Color32::from_rgb(0x3C, 0x46, 0x43)),
    );
    ui.add_space(6.0);
}

/// 白色圆角卡片，里面一行一行放设置。
pub fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(18, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// 卡片里的一行：左边标题（可选说明），右边控件；返回右边控件有没有改动。
///
/// 右边是从右往左排的，所以控件按「右、左」的顺序往里加（见 [`stepper`]）。
pub fn row(
    ui: &mut Ui,
    label: &str,
    desc: Option<&str>,
    right: impl FnOnce(&mut Ui) -> bool,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(label).size(14.0).color(TEXT));
            if let Some(desc) = desc {
                ui.add_space(1.0);
                ui.label(RichText::new(desc).size(12.0).color(MUTED));
            }
            ui.add_space(8.0);
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            changed = right(ui);
        });
    });
    changed
}

/// 卡片里行与行之间的细分隔线。
pub fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, LINE));
}

/// 右边那种拨动开关；返回有没有被点过。
pub fn switch(ui: &mut Ui, on: &mut bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(42.0, 24.0), Sense::click());
    if response.clicked() {
        *on = !*on;
    }
    let t = ui.ctx().animate_bool_with_time(response.id, *on, 0.12);
    let track = if *on { ACCENT } else { TRACK_OFF };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(12), track);
    let x = egui::lerp((rect.left() + 3.0)..=(rect.right() - 21.0), t);
    ui.painter()
        .circle_filled(egui::pos2(x + 9.0, rect.center().y), 9.0, Color32::WHITE);
    response.clicked()
}

/// 「9 − +」这种整数步进；返回有没有改动。调用处是从右往左排的，所以先加加号。
pub fn stepper(ui: &mut Ui, value: &mut i32, min: i32, max: i32, step: i32) -> bool {
    let mut changed = false;
    if round_button(ui, "+", *value < max).clicked() && *value < max {
        *value = (*value + step).min(max);
        changed = true;
    }
    if round_button(ui, "−", *value > min).clicked() && *value > min {
        *value = (*value - step).max(min);
        changed = true;
    }
    ui.label(RichText::new(value.to_string()).size(14.0).color(TEXT));
    changed
}

/// 下拉选择：带边框的小框，里面是当前值和箭头；返回有没有改动。
pub fn select(ui: &mut Ui, value: &mut String, options: &[(&str, &str)]) -> bool {
    let shown = options
        .iter()
        .find(|(key, _)| key == value)
        .map_or("", |(_, label)| *label);
    let galley = ui
        .painter()
        .layout_no_wrap(shown.to_owned(), FontId::proportional(14.0), TEXT);
    let size = Vec2::new((galley.size().x + 44.0).max(76.0), 30.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let stroke = if response.hovered() {
        Stroke::new(1.0, ACCENT)
    } else {
        Stroke::new(1.0, BORDER)
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(8),
        CARD,
        stroke,
        StrokeKind::Inside,
    );
    ui.painter().galley(
        egui::pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
        galley,
        TEXT,
    );
    chevron(
        ui.painter(),
        egui::pos2(rect.right() - 14.0, rect.center().y),
        false,
    );

    let mut changed = false;
    egui::Popup::menu(&response).show(|ui| {
        ui.set_min_width(rect.width());
        for (key, label) in options {
            if ui
                .selectable_label(key == value, RichText::new(*label).size(13.0))
                .clicked()
            {
                *value = (*key).to_owned();
                changed = true;
            }
        }
    });
    changed
}

/// 单行输入框（快捷键、路径、自定义词都用它）；返回有没有改动。
pub fn text_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> bool {
    ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(RichText::new(hint).size(12.0))
            .desired_width(width)
            .font(FontId::proportional(13.0)),
    )
    .changed()
}

/// 单选行（词库那种）：左边圆点、标题和说明，右边可以放按钮；返回有没有被点。
///
/// 行高只由两行字定，圆点和右边的按钮都按这个高度垂直居中——先放文字再画圆点，
/// 否则圆点会跟标题对齐、按钮会被撑到行外。
pub fn radio_row(
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    selected: bool,
    trailing: impl FnOnce(&mut Ui),
) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.add_space(32.0);
        let text = ui
            .vertical(|ui| {
                ui.add_space(8.0);
                ui.label(RichText::new(title).size(14.0).color(TEXT));
                ui.add_space(1.0);
                ui.label(RichText::new(subtitle).size(12.0).color(MUTED));
                ui.add_space(8.0);
            })
            .response;
        let center = egui::pos2(text.rect.left() - 21.0, text.rect.center().y);
        if selected {
            ui.painter()
                .circle_stroke(center, 7.5, Stroke::new(2.0, ACCENT));
            ui.painter().circle_filled(center, 4.0, ACCENT);
        } else {
            ui.painter()
                .circle_stroke(center, 7.5, Stroke::new(1.5, BORDER));
        }
        // 圆点加文字这块能点，右边的按钮不算
        let hit = Rect::from_min_max(egui::pos2(center.x - 11.0, text.rect.top()), text.rect.max);
        clicked = ui
            .interact(hit, ui.scope_id().with(("radio", title)), Sense::click())
            .clicked();

        let size = Vec2::new(ui.available_width(), text.rect.height());
        ui.allocate_ui_with_layout(
            size,
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| trailing(ui),
        );
    });
    clicked
}

/// 可折叠的小标题行（翻页键、模糊音规则、小技巧）；返回有没有被点开／收起。
pub fn collapsible(ui: &mut Ui, label: &str, open: &mut bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
    ui.painter().text(
        egui::pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        TEXT,
    );
    chevron(
        ui.painter(),
        egui::pos2(rect.right() - 8.0, rect.center().y),
        *open,
    );
    if response.clicked() {
        *open = !*open;
        return true;
    }
    false
}

/// 绿色实心主按钮。
pub fn primary_button(ui: &mut Ui, text: &str) -> bool {
    button(ui, text, 104.0, ACCENT, Color32::WHITE)
}

/// 浅灰次按钮。
pub fn secondary_button(ui: &mut Ui, text: &str) -> bool {
    button(ui, text, 84.0, BUTTON_BG, TEXT)
}

/// 左边一行的导航；返回有没有被点。
pub fn nav(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
    let background = if selected {
        ACCENT
    } else if response.hovered() {
        Color32::from_rgb(0xEC, 0xF4, 0xF0)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(8), background);
    let color = if selected { Color32::WHITE } else { TEXT };
    let icon_rect = Rect::from_center_size(
        egui::pos2(rect.left() + 20.0, rect.center().y),
        Vec2::splat(16.0),
    );
    paint_icon(ui.painter(), icon_rect, icon, color);
    ui.painter().text(
        egui::pos2(rect.left() + 38.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        color,
    );
    response.clicked()
}

/// 侧栏顶上的 logo：品牌图和两行产品名。
///
/// 贴图在启动时解一次（1254² 缩到 60² 够 2 倍屏用），别每帧重传。
pub fn brand_logo(ctx: &egui::Context) -> egui::TextureHandle {
    let png = include_bytes!("../../brand.png");
    let image = image::load_from_memory(png).expect("brand.png 坏了");
    let small = image.resize_exact(60, 60, image::imageops::FilterType::Triangle);
    let rgba = small.to_rgba8();
    ctx.load_texture(
        "brand",
        egui::ColorImage::from_rgba_unmultiplied([60, 60], rgba.as_raw()),
        egui::TextureOptions::LINEAR,
    )
}

/// 侧栏顶上的 logo：品牌图加两行产品名。
///
/// 图比两行字矮，所以画完再按整行的中心放，不靠布局对齐。
pub fn logo(ui: &mut Ui, brand: &egui::TextureHandle) {
    let mut slot = Rect::NOTHING;
    let row = ui
        .horizontal(|ui| {
            ui.add_space(6.0);
            slot = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover()).0;
            ui.add_space(2.0);
            ui.vertical(|ui| {
                ui.add_space(1.0);
                ui.label(RichText::new("Flype").size(15.0).strong().color(TEXT));
                ui.label(RichText::new("设置").size(11.0).color(MUTED));
            });
        })
        .response
        .rect;
    let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    let rect = Rect::from_center_size(
        egui::pos2(slot.center().x, row.center().y),
        Vec2::splat(30.0),
    );
    ui.painter().image(brand.id(), rect, uv, Color32::WHITE);
}

/// 定宽圆角按钮，主按钮和次按钮只在这里换颜色。
fn button(ui: &mut Ui, text: &str, width: f32, fill: Color32, color: Color32) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::click());
    let fill = if response.hovered() {
        fill.linear_multiply(0.92)
    } else {
        fill
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(13.5),
        color,
    );
    response.clicked()
}

/// 步进器上的圆形加减按钮，到边界时变淡。
fn round_button(ui: &mut Ui, glyph: &str, enabled: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    let background = if response.hovered() && enabled {
        Color32::from_rgb(0xE2, 0xEA, 0xE6)
    } else {
        BUTTON_BG
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(12), background);
    let color = if enabled { TEXT } else { MUTED };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(14.0),
        color,
    );
    response
}

/// 折叠标题右边的箭头：展开朝上，收起朝下。
fn chevron(painter: &Painter, center: egui::Pos2, open: bool) {
    let stroke = Stroke::new(1.5, MUTED);
    let dy = if open { -2.0 } else { 2.0 };
    painter.line_segment(
        [center + Vec2::new(-4.0, -dy), center + Vec2::new(0.0, dy)],
        stroke,
    );
    painter.line_segment(
        [center + Vec2::new(0.0, dy), center + Vec2::new(4.0, -dy)],
        stroke,
    );
}

/// 导航图标全部用线段和圆画，换个字体也不会变成方块。
fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    let center = rect.center();
    match icon {
        Icon::General => {
            painter.circle_stroke(center, 4.8, stroke);
            painter.circle_filled(center, 1.5, color);
            for step in 0..8 {
                let angle = std::f32::consts::TAU * step as f32 / 8.0;
                let direction = Vec2::new(angle.cos(), angle.sin());
                painter.line_segment([center + direction * 6.0, center + direction * 7.6], stroke);
            }
        }
        Icon::Input => {
            let body = Rect::from_center_size(center, Vec2::new(15.0, 11.0));
            painter.rect_stroke(body, CornerRadius::same(3), stroke, StrokeKind::Inside);
            for step in 0..3 {
                let x = body.left() + 4.0 + step as f32 * 3.5;
                painter.circle_filled(egui::pos2(x, center.y), 1.0, color);
            }
        }
        Icon::Voice => {
            let body =
                Rect::from_center_size(egui::pos2(center.x, center.y - 2.0), Vec2::new(6.0, 10.0));
            painter.rect_stroke(body, CornerRadius::same(3), stroke, StrokeKind::Inside);
            painter.line_segment(
                [
                    egui::pos2(center.x - 4.5, center.y + 1.0),
                    egui::pos2(center.x + 4.5, center.y + 1.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y + 1.0),
                    egui::pos2(center.x, center.y + 6.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - 3.0, center.y + 6.0),
                    egui::pos2(center.x + 3.0, center.y + 6.0),
                ],
                stroke,
            );
        }
        Icon::Emoji => {
            painter.circle_stroke(center, 7.0, stroke);
            painter.circle_filled(egui::pos2(center.x - 2.6, center.y - 2.0), 1.1, color);
            painter.circle_filled(egui::pos2(center.x + 2.6, center.y - 2.0), 1.1, color);
            painter.line_segment(
                [
                    egui::pos2(center.x - 3.0, center.y + 2.6),
                    egui::pos2(center.x, center.y + 4.2),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y + 4.2),
                    egui::pos2(center.x + 3.0, center.y + 2.6),
                ],
                stroke,
            );
        }
        Icon::Book => {
            let body = Rect::from_center_size(center, Vec2::new(14.0, 12.0));
            painter.rect_stroke(body, CornerRadius::same(2), stroke, StrokeKind::Inside);
            painter.line_segment(
                [
                    egui::pos2(body.left() + 3.5, body.top() + 1.0),
                    egui::pos2(body.left() + 3.5, body.bottom() - 1.0),
                ],
                stroke,
            );
        }
    }
}
