use std::io::{Cursor, Error};
use std::sync::Arc;

use ab_glyph::{FontRef, PxScale};
use imageproc::drawing::{draw_filled_rect_mut, draw_line_segment_mut, draw_text_mut, text_size};
use imageproc::image;
use imageproc::image::{ImageFormat, Rgb};
use imageproc::rect::Rect;
use log::error;
use tokio::sync::Mutex;

use crate::config::time::{convert_time_local, get_current_millis};
use crate::config::{FONT, SERVER_CONFIG};
use crate::database::{generate_uuid, Database};
use crate::http::request::Request;
use crate::ip::ip_info::IPInfo;
use crate::results;
use crate::results::TelemetryData;

pub async fn record_result(
    request: &Request,
    database: &mut Arc<Mutex<dyn Database + Send>>,
) -> std::io::Result<String> {
    let default = "".to_string();
    let mut ip_address = request.remote_addr.to_string();
    let mut isp_info = request.form_data.get("ispinfo").unwrap_or(&default).clone();
    let extra = request.form_data.get("extra").unwrap_or(&default);
    let ua = request.headers.get("User-Agent").unwrap_or(&default);
    let lang = request.headers.get("Accept-Language").unwrap_or(&default);
    let dl = request.form_data.get("dl").unwrap_or(&default);
    let ul = request.form_data.get("ul").unwrap_or(&default);
    let ping = request.form_data.get("ping").unwrap_or(&default);
    let jitter = request.form_data.get("jitter").unwrap_or(&default);
    let mut log = request.form_data.get("log").unwrap_or(&default).clone();
    let uuid = generate_uuid();

    let config = SERVER_CONFIG.get().unwrap();
    if config.redact_ip_addresses {
        ip_address = "0.0.0.0".to_string();
        results::redact_hostname(&mut isp_info, "\"hostname\":\"REDACTED\"");
        results::redact_all_ips(&mut isp_info, "0.0.0.0");
        results::redact_hostname(&mut log, "\"hostname\":\"REDACTED\"");
        results::redact_all_ips(&mut log, "0.0.0.0");
    }

    let mut database = database.lock().await;
    let insert_db = database.insert(TelemetryData {
        ip_address,
        isp_info: isp_info.to_string(),
        extra: extra.to_string(),
        user_agent: ua.to_string(),
        lang: lang.to_string(),
        download: dl.to_string(),
        upload: ul.to_string(),
        ping: ping.to_string(),
        jitter: jitter.to_string(),
        log: log.to_string(),
        uuid: uuid.to_string(),
        timestamp: get_current_millis(),
    });
    match insert_db {
        Ok(_) => Ok(uuid),
        Err(e) => Err(Error::other(e)),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TelemetryRenderText {
    provider: Option<String>,
    context: Option<String>,
    ip_address: Option<String>,
    timestamp: String,
    attribution: String,
    footer: String,
}

const RESULT_CARD_WIDTH: u32 = 1200;
const RESULT_CARD_HEIGHT: u32 = 720;
const RESULT_CARD_SAFE_MARGIN: i32 = 48;
const RESULT_CARD_COLUMN_GAP: i32 = 24;
const RESULT_CARD_ROW_GAP: i32 = 20;
const RESULT_CARD_PANEL_PADDING: i32 = 28;
const RESULT_CARD_HEADER_HEIGHT: u32 = 108;
const RESULT_CARD_SECONDARY_HEIGHT: u32 = 132;
const RESULT_CARD_FOOTER_HEIGHT: u32 = 96;
const RESULT_CARD_RADIUS: i32 = 30;

fn normalize_display_text(value: &str, max_chars: usize) -> Option<String> {
    let mut normalized = String::with_capacity(value.len());
    let mut last_was_space = false;

    for ch in value.chars() {
        if ch.is_control() || ch.is_whitespace() {
            if !last_was_space && !normalized.is_empty() {
                normalized.push(' ');
                last_was_space = true;
            }
            continue;
        }

        normalized.push(ch);
        last_was_space = false;
    }

    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.chars().take(max_chars).collect())
}

fn join_display_text(parts: &[Option<String>], separator: &str, max_chars: usize) -> String {
    let joined = parts
        .iter()
        .filter_map(|part| part.as_deref())
        .collect::<Vec<_>>()
        .join(separator);
    joined.chars().take(max_chars).collect()
}

fn build_location_text(isp_info: &IPInfo) -> Option<String> {
    let mut parts = Vec::new();

    for field in [
        &isp_info.rawIspInfo.city,
        &isp_info.rawIspInfo.region,
        &isp_info.rawIspInfo.country,
    ] {
        if let Some(value) = normalize_display_text(field, 32) {
            parts.push(value);
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", ").chars().take(64).collect())
    }
}

fn build_extra_context_text(extra: &str) -> Option<String> {
    let parsed_extra = serde_json::from_str::<serde_json::Value>(extra).ok()?;
    let server_name = parsed_extra.get("server")?.as_str()?;
    normalize_display_text(server_name, 48)
}

fn map_render_text(data: &TelemetryData) -> TelemetryRenderText {
    let parsed_isp_info = serde_json::from_str::<IPInfo>(&data.isp_info).ok();
    let provider = parsed_isp_info
        .as_ref()
        .and_then(|isp_info| normalize_display_text(&isp_info.processedString, 48));
    let context = parsed_isp_info
        .as_ref()
        .and_then(build_location_text)
        .or_else(|| build_extra_context_text(&data.extra));
    let ip_address = normalize_display_text(&data.ip_address, 28);
    let footer = join_display_text(
        &[provider.clone(), context.clone(), ip_address.clone()],
        " | ",
        96,
    );

    TelemetryRenderText {
        provider,
        context,
        ip_address,
        timestamp: convert_time_local(data.timestamp),
        attribution: "Nexio x LibreSpeed".to_string(),
        footer,
    }
}

#[derive(Clone, Copy)]
struct ResultCardLayout {
    header_rect: Rect,
    download_rect: Rect,
    upload_rect: Rect,
    ping_rect: Rect,
    jitter_rect: Rect,
    footer_rect: Rect,
    header_title_scale: f32,
    header_subtitle_scale: f32,
    header_timestamp_scale: f32,
    primary_label_scale: f32,
    primary_value_scale: f32,
    secondary_label_scale: f32,
    secondary_value_scale: f32,
    unit_scale: f32,
    footer_title_scale: f32,
    footer_meta_scale: f32,
    footer_small_scale: f32,
}

fn build_card_layout() -> ResultCardLayout {
    let inner_width = RESULT_CARD_WIDTH as i32 - (RESULT_CARD_SAFE_MARGIN * 2);
    let inner_height = RESULT_CARD_HEIGHT as i32 - (RESULT_CARD_SAFE_MARGIN * 2);
    let column_width = (inner_width - RESULT_CARD_COLUMN_GAP) / 2;
    let header_top = RESULT_CARD_SAFE_MARGIN;
    let primary_top = header_top + RESULT_CARD_HEADER_HEIGHT as i32 + RESULT_CARD_ROW_GAP;
    let primary_height = inner_height
        - RESULT_CARD_HEADER_HEIGHT as i32
        - RESULT_CARD_SECONDARY_HEIGHT as i32
        - RESULT_CARD_FOOTER_HEIGHT as i32
        - (RESULT_CARD_ROW_GAP * 3);
    let secondary_top = primary_top + primary_height + RESULT_CARD_ROW_GAP;
    let footer_top = secondary_top + RESULT_CARD_SECONDARY_HEIGHT as i32 + RESULT_CARD_ROW_GAP;

    ResultCardLayout {
        header_rect: Rect::at(RESULT_CARD_SAFE_MARGIN, header_top)
            .of_size(inner_width as u32, RESULT_CARD_HEADER_HEIGHT),
        download_rect: Rect::at(RESULT_CARD_SAFE_MARGIN, primary_top)
            .of_size(column_width as u32, primary_height as u32),
        upload_rect: Rect::at(
            RESULT_CARD_SAFE_MARGIN + column_width + RESULT_CARD_COLUMN_GAP,
            primary_top,
        )
        .of_size(column_width as u32, primary_height as u32),
        ping_rect: Rect::at(RESULT_CARD_SAFE_MARGIN, secondary_top)
            .of_size(column_width as u32, RESULT_CARD_SECONDARY_HEIGHT),
        jitter_rect: Rect::at(
            RESULT_CARD_SAFE_MARGIN + column_width + RESULT_CARD_COLUMN_GAP,
            secondary_top,
        )
        .of_size(column_width as u32, RESULT_CARD_SECONDARY_HEIGHT),
        footer_rect: Rect::at(RESULT_CARD_SAFE_MARGIN, footer_top)
            .of_size(inner_width as u32, RESULT_CARD_FOOTER_HEIGHT),
        header_title_scale: 36.0,
        header_subtitle_scale: 19.0,
        header_timestamp_scale: 22.0,
        primary_label_scale: 28.0,
        primary_value_scale: 92.0,
        secondary_label_scale: 24.0,
        secondary_value_scale: 54.0,
        unit_scale: 24.0,
        footer_title_scale: 26.0,
        footer_meta_scale: 18.0,
        footer_small_scale: 16.0,
    }
}

struct ImageTheme {
    background_top: Rgb<u8>,
    background_bottom: Rgb<u8>,
    header_surface: Rgb<u8>,
    panel_surface: Rgb<u8>,
    panel_surface_alt: Rgb<u8>,
    footer_surface: Rgb<u8>,
    panel_border: Rgb<u8>,
    text_primary: Rgb<u8>,
    text_secondary: Rgb<u8>,
    text_muted: Rgb<u8>,
    download_accent: Rgb<u8>,
    upload_accent: Rgb<u8>,
    ping_accent: Rgb<u8>,
    jitter_accent: Rgb<u8>,
    brand_accent: Rgb<u8>,
}

fn get_theme(_is_dark: bool) -> ImageTheme {
    ImageTheme {
        background_top: Rgb([8, 12, 28]),
        background_bottom: Rgb([18, 20, 42]),
        header_surface: Rgb([26, 31, 57]),
        panel_surface: Rgb([18, 23, 45]),
        panel_surface_alt: Rgb([21, 27, 51]),
        footer_surface: Rgb([20, 24, 47]),
        panel_border: Rgb([74, 82, 126]),
        text_primary: Rgb([244, 247, 255]),
        text_secondary: Rgb([192, 200, 226]),
        text_muted: Rgb([126, 137, 176]),
        download_accent: Rgb([90, 220, 244]),
        upload_accent: Rgb([191, 166, 255]),
        ping_accent: Rgb([255, 214, 86]),
        jitter_accent: Rgb([255, 160, 186]),
        brand_accent: Rgb([128, 233, 255]),
    }
}

fn lerp_channel(from: u8, to: u8, amount: f32) -> u8 {
    ((from as f32) + ((to as f32) - (from as f32)) * amount)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn lerp_color(from: Rgb<u8>, to: Rgb<u8>, amount: f32) -> Rgb<u8> {
    Rgb([
        lerp_channel(from[0], to[0], amount),
        lerp_channel(from[1], to[1], amount),
        lerp_channel(from[2], to[2], amount),
    ])
}

fn blend_pixel(base: Rgb<u8>, overlay: Rgb<u8>, alpha: f32) -> Rgb<u8> {
    let clamped = alpha.clamp(0.0, 1.0);
    let inverse = 1.0 - clamped;
    Rgb([
        ((base[0] as f32 * inverse) + (overlay[0] as f32 * clamped))
            .round()
            .clamp(0.0, 255.0) as u8,
        ((base[1] as f32 * inverse) + (overlay[1] as f32 * clamped))
            .round()
            .clamp(0.0, 255.0) as u8,
        ((base[2] as f32 * inverse) + (overlay[2] as f32 * clamped))
            .round()
            .clamp(0.0, 255.0) as u8,
    ])
}

fn draw_glow(img: &mut image::RgbImage, center_x: i32, center_y: i32, radius: i32, color: Rgb<u8>) {
    let left = (center_x - radius).max(0) as u32;
    let top = (center_y - radius).max(0) as u32;
    let right = (center_x + radius).min(img.width() as i32 - 1) as u32;
    let bottom = (center_y + radius).min(img.height() as i32 - 1) as u32;

    for y in top..=bottom {
        for x in left..=right {
            let dx = x as f32 - center_x as f32;
            let dy = y as f32 - center_y as f32;
            let distance = (dx * dx + dy * dy).sqrt();
            let normalized = distance / radius as f32;
            if normalized >= 1.0 {
                continue;
            }

            let alpha = (1.0 - normalized).powi(2) * 0.45;
            let pixel = *img.get_pixel(x, y);
            img.put_pixel(x, y, blend_pixel(pixel, color, alpha));
        }
    }
}

fn draw_rounded_rect_mut(img: &mut image::RgbImage, rect: Rect, radius: i32, color: Rgb<u8>) {
    let clamped_radius = radius
        .min(rect.width() as i32 / 2)
        .min(rect.height() as i32 / 2)
        .max(0);
    let inner_width = rect.width() as i32 - (clamped_radius * 2);
    let inner_height = rect.height() as i32 - (clamped_radius * 2);

    if inner_width > 0 {
        draw_filled_rect_mut(
            img,
            Rect::at(rect.left() + clamped_radius, rect.top())
                .of_size(inner_width as u32, rect.height()),
            color,
        );
    }

    if inner_height > 0 {
        draw_filled_rect_mut(
            img,
            Rect::at(rect.left(), rect.top() + clamped_radius)
                .of_size(rect.width(), inner_height as u32),
            color,
        );
    }

    for (x, y) in [
        (rect.left() + clamped_radius, rect.top() + clamped_radius),
        (
            rect.right() - clamped_radius - 1,
            rect.top() + clamped_radius,
        ),
        (
            rect.left() + clamped_radius,
            rect.bottom() - clamped_radius - 1,
        ),
        (
            rect.right() - clamped_radius - 1,
            rect.bottom() - clamped_radius - 1,
        ),
    ] {
        imageproc::drawing::draw_filled_circle_mut(img, (x, y), clamped_radius, color);
    }
}

fn draw_text_right_aligned(
    img: &mut image::RgbImage,
    color: Rgb<u8>,
    right: i32,
    y: i32,
    scale: f32,
    font: &FontRef,
    text: &str,
) {
    let (width, _) = text_size(PxScale::from(scale), font, text);
    draw_text_mut(
        img,
        color,
        right - width as i32,
        y,
        PxScale::from(scale),
        font,
        text,
    );
}

fn fit_text_to_width(font: &FontRef, text: &str, scale: f32, max_width: u32) -> String {
    if text.is_empty() {
        return String::new();
    }

    if text_size(PxScale::from(scale), font, text).0 <= max_width {
        return text.to_string();
    }

    let ellipsis = "...";
    let mut fitted = String::with_capacity(text.len());
    for ch in text.chars() {
        fitted.push(ch);
        let candidate = format!("{fitted}{ellipsis}");
        if text_size(PxScale::from(scale), font, &candidate).0 > max_width {
            fitted.pop();
            break;
        }
    }

    if fitted.is_empty() {
        ellipsis.to_string()
    } else {
        format!("{fitted}{ellipsis}")
    }
}

fn draw_metric_badge(
    img: &mut image::RgbImage,
    center_x: i32,
    center_y: i32,
    accent: Rgb<u8>,
    is_download: bool,
) {
    imageproc::drawing::draw_filled_circle_mut(img, (center_x, center_y), 18, Rgb([13, 17, 34]));
    draw_line_segment_mut(
        img,
        ((center_x - 11) as f32, center_y as f32),
        ((center_x + 11) as f32, center_y as f32),
        accent,
    );
    let tip_y = if is_download {
        center_y + 9
    } else {
        center_y - 9
    };
    let shaft_top = if is_download {
        center_y - 9
    } else {
        center_y + 9
    };
    draw_line_segment_mut(
        img,
        (center_x as f32, shaft_top as f32),
        (center_x as f32, tip_y as f32),
        accent,
    );
    draw_line_segment_mut(
        img,
        (
            (center_x - 6) as f32,
            (center_y + if is_download { 3 } else { -3 }) as f32,
        ),
        (center_x as f32, tip_y as f32),
        accent,
    );
    draw_line_segment_mut(
        img,
        (
            (center_x + 6) as f32,
            (center_y + if is_download { 3 } else { -3 }) as f32,
        ),
        (center_x as f32, tip_y as f32),
        accent,
    );
}

fn draw_signal_badge(
    img: &mut image::RgbImage,
    center_x: i32,
    center_y: i32,
    accent: Rgb<u8>,
    is_jitter: bool,
) {
    imageproc::drawing::draw_filled_circle_mut(img, (center_x, center_y), 16, Rgb([13, 17, 34]));
    draw_line_segment_mut(
        img,
        ((center_x - 8) as f32, (center_y + 5) as f32),
        ((center_x - 2) as f32, (center_y - 2) as f32),
        accent,
    );
    draw_line_segment_mut(
        img,
        ((center_x - 2) as f32, (center_y - 2) as f32),
        ((center_x + 2) as f32, (center_y + 3) as f32),
        accent,
    );
    draw_line_segment_mut(
        img,
        ((center_x + 2) as f32, (center_y + 3) as f32),
        ((center_x + 8) as f32, (center_y - 6) as f32),
        accent,
    );
    if is_jitter {
        draw_line_segment_mut(
            img,
            ((center_x - 9) as f32, (center_y - 9) as f32),
            ((center_x + 9) as f32, (center_y - 9) as f32),
            accent,
        );
    }
}

fn draw_panel(img: &mut image::RgbImage, rect: Rect, fill: Rgb<u8>, accent: Rgb<u8>) {
    draw_rounded_rect_mut(img, rect, RESULT_CARD_RADIUS, fill);

    draw_filled_rect_mut(
        img,
        Rect::at(rect.left(), rect.top()).of_size(rect.width(), 6),
        accent,
    );

    draw_line_segment_mut(
        img,
        ((rect.left() + 18) as f32, (rect.bottom() - 1) as f32),
        ((rect.right() - 18) as f32, (rect.bottom() - 1) as f32),
        accent,
    );
}

fn draw_portal_background(img: &mut image::RgbImage, theme: &ImageTheme) {
    for y in 0..img.height() {
        let blend = y as f32 / (img.height() - 1) as f32;
        let row_color = lerp_color(theme.background_top, theme.background_bottom, blend);
        for x in 0..img.width() {
            img.put_pixel(x, y, row_color);
        }
    }

    draw_glow(img, 170, 120, 280, theme.download_accent);
    draw_glow(img, 980, 140, 320, theme.upload_accent);
    draw_glow(img, 1040, 620, 260, theme.jitter_accent);
}

fn draw_primary_metric_panel(
    img: &mut image::RgbImage,
    rect: Rect,
    label: &str,
    value: &str,
    accent: Rgb<u8>,
    font: &FontRef,
    layout: &ResultCardLayout,
    theme: &ImageTheme,
    is_download: bool,
) {
    draw_panel(img, rect, theme.panel_surface, accent);

    let badge_x = rect.left() + RESULT_CARD_PANEL_PADDING + 18;
    let badge_y = rect.top() + RESULT_CARD_PANEL_PADDING + 18;
    draw_metric_badge(img, badge_x, badge_y, accent, is_download);

    let label_text = fit_text_to_width(
        font,
        label,
        layout.primary_label_scale,
        rect.width() - ((RESULT_CARD_PANEL_PADDING + 64) as u32),
    );
    draw_text_mut(
        img,
        theme.text_secondary,
        rect.left() + RESULT_CARD_PANEL_PADDING + 48,
        rect.top() + RESULT_CARD_PANEL_PADDING - 2,
        PxScale::from(layout.primary_label_scale),
        font,
        &label_text,
    );

    let value_text = fit_text_to_width(
        font,
        value,
        layout.primary_value_scale,
        rect.width() - (RESULT_CARD_PANEL_PADDING as u32 * 2),
    );
    draw_text_mut(
        img,
        accent,
        rect.left() + RESULT_CARD_PANEL_PADDING,
        rect.top() + 78,
        PxScale::from(layout.primary_value_scale),
        font,
        &value_text,
    );

    draw_text_mut(
        img,
        theme.text_muted,
        rect.left() + RESULT_CARD_PANEL_PADDING,
        rect.top() + rect.height() as i32 - 56,
        PxScale::from(layout.unit_scale),
        font,
        "Mbps",
    );
}

fn draw_secondary_metric_panel(
    img: &mut image::RgbImage,
    rect: Rect,
    label: &str,
    value: &str,
    accent: Rgb<u8>,
    font: &FontRef,
    layout: &ResultCardLayout,
    theme: &ImageTheme,
    is_jitter: bool,
) {
    draw_panel(img, rect, theme.panel_surface_alt, accent);

    let badge_x = rect.left() + RESULT_CARD_PANEL_PADDING + 16;
    let badge_y = rect.top() + 38;
    draw_signal_badge(img, badge_x, badge_y, accent, is_jitter);

    draw_text_mut(
        img,
        theme.text_secondary,
        rect.left() + RESULT_CARD_PANEL_PADDING + 42,
        rect.top() + 18,
        PxScale::from(layout.secondary_label_scale),
        font,
        label,
    );

    let value_text = fit_text_to_width(
        font,
        value,
        layout.secondary_value_scale,
        rect.width() - (RESULT_CARD_PANEL_PADDING as u32 * 2),
    );
    draw_text_mut(
        img,
        accent,
        rect.left() + RESULT_CARD_PANEL_PADDING,
        rect.top() + 54,
        PxScale::from(layout.secondary_value_scale),
        font,
        &value_text,
    );
    draw_text_mut(
        img,
        theme.text_muted,
        rect.left() + RESULT_CARD_PANEL_PADDING,
        rect.top() + rect.height() as i32 - 42,
        PxScale::from(layout.unit_scale - 2.0),
        font,
        "ms",
    );
}

fn draw_header(
    img: &mut image::RgbImage,
    rect: Rect,
    render_text: &TelemetryRenderText,
    font: &FontRef,
    layout: &ResultCardLayout,
    theme: &ImageTheme,
) {
    draw_panel(img, rect, theme.header_surface, theme.brand_accent);

    let brand_rect = Rect::at(rect.left() + 24, rect.top() + 22).of_size(64, 64);
    draw_rounded_rect_mut(img, brand_rect, 18, theme.brand_accent);
    draw_text_mut(
        img,
        Rgb([10, 13, 28]),
        brand_rect.left() + 18,
        brand_rect.top() + 8,
        PxScale::from(38.0),
        font,
        "N",
    );

    draw_text_mut(
        img,
        theme.text_primary,
        rect.left() + 112,
        rect.top() + 20,
        PxScale::from(layout.header_title_scale),
        font,
        "Nexio Speed Test",
    );
    draw_text_mut(
        img,
        theme.text_muted,
        rect.left() + 112,
        rect.top() + 60,
        PxScale::from(layout.header_subtitle_scale),
        font,
        "Portal telemetry result",
    );

    let timestamp_text = fit_text_to_width(
        font,
        &render_text.timestamp,
        layout.header_timestamp_scale,
        320,
    );
    draw_text_right_aligned(
        img,
        theme.text_secondary,
        rect.right() - 24,
        rect.top() + 26,
        layout.header_timestamp_scale,
        font,
        &timestamp_text,
    );
}

fn draw_footer(
    img: &mut image::RgbImage,
    rect: Rect,
    render_text: &TelemetryRenderText,
    font: &FontRef,
    layout: &ResultCardLayout,
    theme: &ImageTheme,
) {
    draw_panel(img, rect, theme.footer_surface, theme.panel_border);

    let provider_text = render_text
        .provider
        .as_deref()
        .or(render_text.context.as_deref())
        .unwrap_or("Telemetry result");
    let provider_text = fit_text_to_width(font, provider_text, layout.footer_title_scale, 700);
    draw_text_mut(
        img,
        theme.text_primary,
        rect.left() + RESULT_CARD_PANEL_PADDING,
        rect.top() + 16,
        PxScale::from(layout.footer_title_scale),
        font,
        &provider_text,
    );

    let context_line = join_display_text(
        &[render_text.context.clone(), render_text.ip_address.clone()],
        " | ",
        96,
    );
    if !context_line.is_empty() {
        let fitted_meta = fit_text_to_width(font, &context_line, layout.footer_meta_scale, 700);
        draw_text_mut(
            img,
            theme.text_muted,
            rect.left() + RESULT_CARD_PANEL_PADDING,
            rect.top() + 52,
            PxScale::from(layout.footer_meta_scale),
            font,
            &fitted_meta,
        );
    }

    draw_text_right_aligned(
        img,
        theme.text_secondary,
        rect.right() - RESULT_CARD_PANEL_PADDING,
        rect.top() + 22,
        layout.footer_meta_scale,
        font,
        &fit_text_to_width(
            font,
            &render_text.attribution,
            layout.footer_meta_scale,
            260,
        ),
    );
    draw_text_right_aligned(
        img,
        theme.text_muted,
        rect.right() - RESULT_CARD_PANEL_PADDING,
        rect.top() + 52,
        layout.footer_small_scale,
        font,
        "Share card",
    );
}

pub fn draw_result(data: &TelemetryData) -> Vec<u8> {
    let mut img = image::RgbImage::new(RESULT_CARD_WIDTH, RESULT_CARD_HEIGHT);
    let font = FONT.get().unwrap();
    let render_text = map_render_text(data);
    let layout = build_card_layout();
    let theme = get_theme(true);

    draw_portal_background(&mut img, &theme);
    draw_header(
        &mut img,
        layout.header_rect,
        &render_text,
        font,
        &layout,
        &theme,
    );
    draw_primary_metric_panel(
        &mut img,
        layout.download_rect,
        "Download",
        &data.download,
        theme.download_accent,
        font,
        &layout,
        &theme,
        true,
    );
    draw_primary_metric_panel(
        &mut img,
        layout.upload_rect,
        "Upload",
        &data.upload,
        theme.upload_accent,
        font,
        &layout,
        &theme,
        false,
    );
    draw_secondary_metric_panel(
        &mut img,
        layout.ping_rect,
        "Ping",
        &data.ping,
        theme.ping_accent,
        font,
        &layout,
        &theme,
        false,
    );
    draw_secondary_metric_panel(
        &mut img,
        layout.jitter_rect,
        "Jitter",
        &data.jitter,
        theme.jitter_accent,
        font,
        &layout,
        &theme,
        true,
    );
    draw_footer(
        &mut img,
        layout.footer_rect,
        &render_text,
        font,
        &layout,
        &theme,
    );

    let mut buffer: Cursor<Vec<u8>> = Cursor::new(Vec::new());
    if let Err(e) = img.write_to(&mut buffer, ImageFormat::Png) {
        error!("Image writer buffer error : {e}")
    }

    buffer.into_inner()
}

fn ensure_result_font() {
    FONT.get_or_init(|| {
        FontRef::try_from_slice(include_bytes!("../../assets/open-sans.ttf")).unwrap()
    });
}

fn sample_result_data() -> TelemetryData {
    TelemetryData {
        ip_address: "2001:db8::42".to_string(),
        isp_info: r#"{"processedString":"Nexio Fiber","rawIspInfo":{"ip":"2001:db8::42","hostname":"example","city":"Amsterdam","region":"Noord-Holland","country":"NL","loc":"","org":"Nexio","postal":"","timezone":"","readme":null}}"#.to_string(),
        extra: r#"{"server":"Amsterdam Edge 1"}"#.to_string(),
        user_agent: "sample-generator".to_string(),
        lang: "en-US".to_string(),
        download: "874.48".to_string(),
        upload: "609.27".to_string(),
        ping: "7.0".to_string(),
        jitter: "29.0".to_string(),
        log: "".to_string(),
        uuid: "sample-result".to_string(),
        timestamp: 1_774_420_740_000,
    }
}

pub fn write_sample_result(path: &str) -> std::io::Result<()> {
    ensure_result_font();
    std::fs::write(path, draw_result(&sample_result_data()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    use imageproc::image::GenericImageView;

    use crate::config::{ServerConfig, FONT, SERVER_CONFIG};

    fn init_test_globals() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            SERVER_CONFIG.get_or_init(|| ServerConfig {
                bind_address: "127.0.0.1".to_string(),
                listen_port: 8080,
                worker_threads: serde_json::json!(1),
                base_url: "/backend".to_string(),
                ipinfo_api_key: "".to_string(),
                stats_password: "".to_string(),
                redact_ip_addresses: false,
                result_image_theme: "light".to_string(),
                assets_path: "".to_string(),
                database_type: "memory".to_string(),
                database_hostname: None,
                database_name: None,
                database_username: None,
                database_password: None,
                database_file: None,
                enable_tls: false,
                tls_cert_file: "".to_string(),
                tls_key_file: "".to_string(),
            });
            FONT.get_or_init(|| {
                FontRef::try_from_slice(include_bytes!("../../assets/open-sans.ttf")).unwrap()
            });
        });
    }

    fn sample_telemetry(isp_info: &str, extra: &str) -> TelemetryData {
        TelemetryData {
            ip_address: "203.0.113.42".to_string(),
            isp_info: isp_info.to_string(),
            extra: extra.to_string(),
            user_agent: "test-agent".to_string(),
            lang: "en-US".to_string(),
            download: "100.0".to_string(),
            upload: "20.0".to_string(),
            ping: "5.0".to_string(),
            jitter: "1.0".to_string(),
            log: "".to_string(),
            uuid: "uuid".to_string(),
            timestamp: 1_700_000_000,
        }
    }

    #[test]
    fn valid_isp_info_yields_provider_and_footer_data() {
        init_test_globals();

        let data = sample_telemetry(
            r#"{"processedString":"Example ISP","rawIspInfo":{"ip":"203.0.113.42","hostname":"host","city":"Amsterdam","region":"Noord-Holland","country":"NL","loc":"","org":"Example","postal":"","timezone":"","readme":null}}"#,
            r#"{"telemetry_extra":"kept-opaque"}"#,
        );

        let render_text = map_render_text(&data);

        assert_eq!(render_text.provider.as_deref(), Some("Example ISP"));
        assert!(render_text.footer.contains("Example ISP"));
        assert!(render_text.footer.contains("Amsterdam"));
        assert!(render_text.footer.contains("203.0.113.42"));
        assert!(!render_text.footer.contains("null"));
        assert!(!render_text.footer.contains("  "));
    }

    #[test]
    fn malformed_isp_info_does_not_panic_mapping() {
        init_test_globals();

        let data = sample_telemetry("{bad json", r#"{"telemetry_extra":"ignored"}"#);

        let render_text = map_render_text(&data);

        assert!(render_text.provider.is_none());
        assert!(render_text.footer.contains("203.0.113.42"));
        assert!(!render_text.footer.contains("null"));
    }

    #[test]
    fn malformed_extra_does_not_panic_mapping() {
        init_test_globals();

        let data = sample_telemetry(
            r#"{"processedString":"Provider","rawIspInfo":{"ip":"203.0.113.42","hostname":"","city":"Amsterdam","region":"","country":"NL","loc":"","org":"","postal":"","timezone":"","readme":null}}"#,
            "{still bad",
        );

        let render_text = map_render_text(&data);

        assert_eq!(render_text.provider.as_deref(), Some("Provider"));
        assert!(render_text.footer.contains("Amsterdam"));
        assert!(render_text.footer.contains("203.0.113.42"));
    }

    #[test]
    fn trusted_extra_server_name_fills_footer_when_location_is_missing() {
        init_test_globals();

        let data = sample_telemetry(
            r#"{"processedString":"Provider","rawIspInfo":{"ip":"203.0.113.42","hostname":"","city":"","region":"","country":"","loc":"","org":"","postal":"","timezone":"","readme":null}}"#,
            r#"{"server":"Amsterdam Edge 1","extra":{"note":"opaque"}} "#,
        );

        let render_text = map_render_text(&data);

        assert_eq!(render_text.provider.as_deref(), Some("Provider"));
        assert!(render_text.footer.contains("Amsterdam Edge 1"));
        assert!(render_text.footer.contains("203.0.113.42"));
    }

    #[test]
    fn long_provider_location_and_ip_strings_collapse_without_placeholder_junk() {
        init_test_globals();

        let long_provider = format!("Provider  \n\t{}\n{}", "X".repeat(120), "Y".repeat(120));
        let data = sample_telemetry(
            &serde_json::json!({
                "processedString": long_provider,
                "rawIspInfo": {
                    "ip": "203.0.113.42",
                    "hostname": "",
                    "city": format!("City\n{}", "A".repeat(120)),
                    "region": format!("Region\t{}", "B".repeat(120)),
                    "country": format!("Country  {}", "C".repeat(120)),
                    "loc": "",
                    "org": "",
                    "postal": "",
                    "timezone": "",
                    "readme": null
                }
            })
            .to_string(),
            r#"{"telemetry_extra":"opaque"}"#,
        );

        let render_text = map_render_text(&data);

        let provider = render_text.provider.as_deref().unwrap();
        assert!(provider.len() <= 48);
        assert!(!provider.contains('\n'));
        assert!(!provider.contains('\t'));
        assert!(!provider.contains("  "));

        assert!(render_text.footer.len() <= 96);
        assert!(!render_text.footer.contains('\n'));
        assert!(!render_text.footer.contains('\t'));
        assert!(!render_text.footer.contains("null"));
        assert!(!render_text.footer.contains("||"));
        assert!(!render_text.footer.starts_with(" | "));
        assert!(!render_text.footer.ends_with(" | "));
    }

    #[test]
    fn draw_result_emits_decodable_image_with_1200x720_dimensions() {
        init_test_globals();

        let bytes = draw_result(&sample_telemetry(
            r#"{"processedString":"Example ISP","rawIspInfo":{"ip":"203.0.113.42","hostname":"","city":"Amsterdam","region":"Noord-Holland","country":"NL","loc":"","org":"Example","postal":"","timezone":"","readme":null}}"#,
            r#"{"server":"Amsterdam Edge 1"}"#,
        ));

        assert!(!bytes.is_empty());

        let image = image::load_from_memory(&bytes).expect("result image should decode");
        assert_eq!(image.dimensions(), (1200, 720));
    }

    #[test]
    fn draw_result_encodes_png_bytes() {
        init_test_globals();

        let bytes = draw_result(&sample_telemetry(
            r#"{"processedString":"Example ISP","rawIspInfo":{"ip":"203.0.113.42","hostname":"","city":"Amsterdam","region":"Noord-Holland","country":"NL","loc":"","org":"Example","postal":"","timezone":"","readme":null}}"#,
            r#"{"server":"Amsterdam Edge 1"}"#,
        ));

        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn draw_result_layout_preserves_outer_safe_margin() {
        let layout = build_card_layout();

        for rect in [
            layout.header_rect,
            layout.download_rect,
            layout.upload_rect,
            layout.ping_rect,
            layout.jitter_rect,
            layout.footer_rect,
        ] {
            assert!(rect.left() >= RESULT_CARD_SAFE_MARGIN as i32);
            assert!(rect.top() >= RESULT_CARD_SAFE_MARGIN as i32);
            assert!(rect.right() <= RESULT_CARD_WIDTH as i32 - RESULT_CARD_SAFE_MARGIN);
            assert!(rect.bottom() <= RESULT_CARD_HEIGHT as i32 - RESULT_CARD_SAFE_MARGIN);
        }
    }

    #[test]
    fn draw_result_fits_long_footer_text_within_footer_band() {
        init_test_globals();

        let layout = build_card_layout();
        let font = FONT.get().unwrap();
        let fitted = fit_text_to_width(
            font,
            &format!(
                "Provider {} | Amsterdam {}, Noord-Holland {} | 2001:db8::1234:5678:90ab:cdef",
                "X".repeat(120),
                "Y".repeat(120),
                "Z".repeat(120),
            ),
            layout.footer_meta_scale,
            layout.footer_rect.width() - (RESULT_CARD_PANEL_PADDING as u32 * 2),
        );

        assert!(
            text_size(PxScale::from(layout.footer_meta_scale), font, &fitted).0
                <= layout.footer_rect.width() - (RESULT_CARD_PANEL_PADDING as u32 * 2)
        );
    }
}
