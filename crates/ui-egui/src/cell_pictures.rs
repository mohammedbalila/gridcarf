//! Cell pictures use the current cell geometry, independent of floating objects.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex, Weak};

use egui::{Color32, Painter, Rect, TextureHandle, pos2, vec2};
use gridcraft_engine::model::CellPicture;

const MAX_ENCODED: usize = 16 * 1024 * 1024;
const MAX_PIXELS: u64 = 16_000_000;
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
// The byte budget controls texture memory; sparse entries have a separate modest bound.
const MAX_CACHE_ITEMS: usize = 4096;

#[derive(Default)]
struct Cache {
    entries: HashMap<usize, Entry>,
    bytes: usize,
}

struct Entry {
    // Holding a Weak prevents this allocation address from being reused. The
    // immutable Arc is the content identity: copy/undo share it; replacement
    // creates another allocation. No source bytes are cloned or hashed per frame.
    source: Weak<CellPicture>,
    texture: Option<TextureHandle>,
    bytes: usize,
    used: u64,
}

impl Cache {
    fn forget_dead(&mut self) {
        self.entries.retain(|_, entry| {
            if entry.source.strong_count() == 0 {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
                false
            } else {
                true
            }
        });
    }

    fn texture(&mut self, ctx: &egui::Context, picture: &Arc<CellPicture>) -> Option<TextureHandle> {
        let frame = ctx.cumulative_frame_nr();
        let key = Arc::as_ptr(picture) as usize;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.used = frame;
            return entry.texture.clone();
        }
        // Prune discarded workbooks/payloads before allocating another texture.
        self.forget_dead();
        // Read only the header before admission. Never decode an image just to evict it
        // again while painting a larger visible set on the next frame.
        let source = dimensions(picture);
        let estimate = source.map_or(0, |(_, (w, h))| w as usize * h as usize * 4);
        while self.entries.len() >= MAX_CACHE_ITEMS || self.bytes.saturating_add(estimate) > MAX_CACHE_BYTES {
            let oldest = self.entries.iter().filter(|(_, e)| e.used.saturating_add(1) < frame).min_by_key(|(_, e)| e.used).map(|(key, _)| *key);
            let Some(oldest) = oldest else {
                // Keep visible images stable under pressure; show the placeholder for
                // excess content instead of re-decoding it each frame.
                return None;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
            }
        }
        let decoded = source.and_then(|(format, _)| decode_format(picture, format));
        let bytes = decoded.as_ref().map_or(0, |image| image.pixels.len() * 4);
        let texture = decoded.map(|image| ctx.load_texture("cell picture", image, egui::TextureOptions::LINEAR));
        self.bytes += bytes;
        self.entries.insert(key, Entry { source: Arc::downgrade(picture), texture: texture.clone(), bytes, used: frame });
        texture
    }
}

fn dimensions(picture: &CellPicture) -> Option<(image::ImageFormat, (u32, u32))> {
    if picture.data.len() > MAX_ENCODED {
        return None;
    }
    let format = image::guess_format(&picture.data).ok()?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return None;
    }
    let dimensions = image::ImageReader::with_format(Cursor::new(&picture.data), format).into_dimensions().ok()?;
    if dimensions.0 == 0
        || dimensions.1 == 0
        || dimensions.0 > 8192
        || dimensions.1 > 8192
        || u64::from(dimensions.0) * u64::from(dimensions.1) > MAX_PIXELS
    {
        return None;
    }
    Some((format, dimensions))
}

#[cfg(test)]
fn decode(picture: &CellPicture) -> Option<egui::ColorImage> {
    let (format, _) = dimensions(picture)?;
    decode_format(picture, format)
}

fn decode_format(picture: &CellPicture, format: image::ImageFormat) -> Option<egui::ColorImage> {
    let mut reader = image::ImageReader::with_format(Cursor::new(&picture.data), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.into_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

fn fitted_rect(cell: Rect, size: [usize; 2]) -> Option<Rect> {
    if cell.width() <= 0.0 || cell.height() <= 0.0 || size.contains(&0) {
        return None;
    }
    let source = vec2(size[0] as f32, size[1] as f32);
    let scale = (cell.width() / source.x).min(cell.height() / source.y);
    Some(Rect::from_center_size(cell.center(), source * scale))
}

/// Drops dead-source textures on logic-only ticks without creating a cache for an unused context.
pub(crate) fn forget_dead(ctx: &egui::Context) {
    let Some(cache) = ctx.data(|data| data.get_temp::<Arc<Mutex<Cache>>>(egui::Id::unique("gridcraft.cell_pictures"))) else { return };
    // Release the context data lock before taking the cache lock, as painting does.
    cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).forget_dead();
}

#[cfg(test)]
pub(crate) fn lifetime_evidence(ctx: &egui::Context) -> Option<[usize; 6]> {
    let cache = ctx.data(|data| data.get_temp::<Arc<Mutex<Cache>>>(egui::Id::unique("gridcraft.cell_pictures")))?;
    let cache = cache.lock().unwrap();
    Some([
        cache.entries.len(),
        cache.entries.capacity(),
        cache.bytes,
        cache.entries.values().filter(|entry| entry.source.strong_count() != 0).count(),
        cache.entries.values().filter(|entry| entry.texture.is_some()).count(),
        cache.entries.capacity() * std::mem::size_of::<(usize, Entry)>(),
    ])
}

/// Draws a cached cell picture fitted inside the cell and clipped to it, or paints the picture fallback icon when no texture is available.
pub(crate) fn paint(painter: &Painter, cell: Rect, picture: &Arc<CellPicture>) {
    if cell.width() <= 0.0 || cell.height() <= 0.0 || !cell.intersects(painter.clip_rect()) {
        return;
    }
    // Context-local handles must never leak between offscreen, native or browser
    // egui contexts, even when a workbook shares its Arc payload with another view.
    let cache = painter.ctx().data_mut(|data| data.get_temp_mut_or_default::<Arc<Mutex<Cache>>>(egui::Id::unique("gridcraft.cell_pictures")).clone());
    let texture = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).texture(painter.ctx(), picture);
    let painter = painter.with_clip_rect(cell.intersect(painter.clip_rect()));
    if let Some(texture) = texture {
        if let Some(rect) = fitted_rect(cell, texture.size()) {
            painter.image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
    } else {
        crate::icons::paint_on_sheet(
            &painter,
            cell.shrink(2.0),
            crate::icons::Icon::Picture,
            crate::theme::Tokens::get(painter.ctx()).sheet_control_border,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_engine::core::CellRef;
    use gridcraft_engine::model::Workbook;
    use gridcraft_engine::{DocState, Session, Trust};
    use serde_json::{Value as Json, json};

    use crate::{Services, SheetApp};

    fn picture(color: [u8; 4]) -> Result<Arc<CellPicture>, image::ImageError> {
        let image = image::RgbaImage::from_pixel(4, 2, image::Rgba(color));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png)?;
        Ok(Arc::new(CellPicture { data: bytes.into_inner(), mime: "image/png".into(), alt: "Test picture".into() }))
    }

    fn picture_app(picture: Arc<CellPicture>) -> (SheetApp, Weak<CellPicture>, u64) {
        let source = Arc::downgrade(&picture);
        let mut wb = Workbook::new();
        wb.sheet_mut(0).unwrap().set_picture(CellRef::new(0, 0), picture);
        let mut session = Session::new();
        session.new_workbook(); // A sentinel keeps clearClipboard enabled after source close.
        session.add_document(DocState::new(wb, None, "Cell picture lifetime".into(), Trust::Local));
        let uid = session.active().unwrap().uid;
        let mut app = SheetApp::new(session, Services::default());
        app.ui.autosave = false;
        (app, source, uid)
    }

    fn run(app: &mut SheetApp, command: &str, params: Json) -> Json {
        let result = app.run(command, params).unwrap();
        assert_eq!(app.session.depth(), 0, "observe ownership after command-local snapshots are dropped");
        result
    }

    fn paint_once(ctx: &egui::Context, app: &SheetApp) {
        let picture = &app.session.active().unwrap().wb.active().unwrap().cell_pictures[&CellRef::new(0, 0)];
        let mut output = ctx.run_ui(Default::default(), |ui| {
            paint(ui.painter(), Rect::from_min_size(pos2(10.0, 10.0), vec2(100.0, 100.0)), picture);
        });
        // No renderer consumes the upload delta in these headless tests.
        output.textures_delta.clear();
    }

    fn cache_state(ctx: &egui::Context) -> Option<(usize, usize, Option<egui::TextureId>)> {
        let cache = ctx.data(|data| data.get_temp::<Arc<Mutex<Cache>>>(egui::Id::unique("gridcraft.cell_pictures")))?;
        let cache = cache.lock().unwrap();
        Some((cache.entries.len(), cache.bytes, cache.entries.values().find_map(|entry| entry.texture.as_ref().map(TextureHandle::id))))
    }

    fn hidden_ticks(ctx: &egui::Context, app: &mut SheetApp) {
        for _ in 0..2 {
            let _ = ctx.run_logic(&egui::RawInput::default(), |ctx| app.logic(ctx));
        }
    }

    fn close_active_document(app: &mut SheetApp) {
        let uid = app.session.active().unwrap().uid;
        assert_eq!(run(app, "file.close", json!({"document": true, "force": true}))["closed"], true);
        assert!(app.session.document_index(uid).is_none());
        assert!(app.session.windows().iter().all(|window| window.doc_uid != uid));
        assert!(!app.session.documents().is_empty(), "the sentinel stays open");
    }

    fn assert_source_dead(source: &Weak<CellPicture>) {
        assert_eq!(source.strong_count(), 0, "release all legitimate source owners before checking the cache");
        assert!(source.upgrade().is_none());
    }

    #[test]
    fn dead_picture_is_pruned_by_logic_without_another_paint() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let (mut app, source, _) = picture_app(picture([255, 0, 0, 255])?);
        paint_once(&ctx, &app);
        let initial = cache_state(&ctx).unwrap();
        assert_eq!((initial.0, initial.1), (1, 32));
        assert!(initial.2.is_some());
        run(&mut app, "cell.set", json!({"cell": "A1", "input": "replacement"}));
        {
            // Fixture isolation only: this is not a public clear-history or save command.
            let d = app.session.active_mut().unwrap();
            assert!(d.wb.active().unwrap().cell_pictures.is_empty());
            d.undo.clear();
            d.redo.clear();
            d.saved = d.wb.clone();
        }
        assert_source_dead(&source);
        hidden_ticks(&ctx, &mut app);
        assert_eq!(cache_state(&ctx), Some((0, 0, None)), "logic must release the last dead texture without another paint");
        hidden_ticks(&ctx, &mut app);
        assert_eq!(cache_state(&ctx), Some((0, 0, None)));
        Ok(())
    }

    #[test]
    fn source_close_prunes_picture_on_hidden_logic() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let (mut app, source, uid) = picture_app(picture([255, 0, 0, 255])?);
        paint_once(&ctx, &app);
        let initial = cache_state(&ctx).unwrap();
        assert_eq!((initial.0, initial.1), (1, 32));
        assert!(initial.2.is_some());
        assert_eq!(app.session.active().unwrap().uid, uid);
        close_active_document(&mut app);
        assert_source_dead(&source);
        hidden_ticks(&ctx, &mut app);
        assert_eq!(cache_state(&ctx), Some((0, 0, None)), "closed source textures must be pruned on hidden ticks");
        Ok(())
    }

    #[test]
    fn undo_and_saved_sources_keep_picture_until_released() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let (mut app, source, _) = picture_app(picture([255, 0, 0, 255])?);
        paint_once(&ctx, &app);
        let initial = cache_state(&ctx).unwrap();
        assert_eq!((initial.0, initial.1), (1, 32));
        assert!(initial.2.is_some());
        run(&mut app, "cell.set", json!({"cell": "A1", "input": "replacement"}));
        {
            let d = app.session.active().unwrap();
            assert!(d.wb.active().unwrap().cell_pictures.is_empty());
            assert!(d.undo.iter().any(|entry| entry.wb.active().unwrap().cell_pictures.contains_key(&CellRef::new(0, 0))));
        }
        hidden_ticks(&ctx, &mut app);
        assert!(source.upgrade().is_some());
        assert_eq!(cache_state(&ctx), Some(initial), "undo/saved owners keep the same texture");
        run(&mut app, "edit.undo", json!({}));
        assert_eq!(Arc::as_ptr(&app.session.active().unwrap().wb.active().unwrap().cell_pictures[&CellRef::new(0, 0)]), source.as_ptr());
        run(&mut app, "edit.redo", json!({}));
        {
            // Isolate the saved owner; normal user commands still retain history.
            let d = app.session.active_mut().unwrap();
            d.undo.clear();
            d.redo.clear();
            assert!(d.wb.active().unwrap().cell_pictures.is_empty());
            assert_eq!(Arc::as_ptr(&d.saved.active().unwrap().cell_pictures[&CellRef::new(0, 0)]), source.as_ptr());
        }
        hidden_ticks(&ctx, &mut app);
        assert!(source.upgrade().is_some());
        assert_eq!(cache_state(&ctx), Some(initial), "saved alone keeps the same texture");
        {
            // Fixture isolation, not a successful public save.
            let d = app.session.active_mut().unwrap();
            d.saved = d.wb.clone();
        }
        assert_source_dead(&source);
        hidden_ticks(&ctx, &mut app);
        assert_eq!(cache_state(&ctx), Some((0, 0, None)), "prune only after the final restoring source is released");
        Ok(())
    }

    #[test]
    fn clipboard_source_survives_close_and_paste_until_released() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let (mut app, source, _) = picture_app(picture([255, 0, 0, 255])?);
        paint_once(&ctx, &app);
        let initial = cache_state(&ctx).unwrap();
        assert_eq!((initial.0, initial.1), (1, 32));
        assert!(initial.2.is_some());
        run(&mut app, "edit.copy", json!({"range": "A1"}));
        close_active_document(&mut app);
        hidden_ticks(&ctx, &mut app);
        assert!(source.upgrade().is_some());
        assert_eq!(cache_state(&ctx), Some(initial), "source-closed clipboard still owns the image");
        app.session.new_workbook();
        run(&mut app, "edit.paste", json!({"at": "A1"}));
        {
            let pasted = &app.session.active().unwrap().wb.active().unwrap().cell_pictures[&CellRef::new(0, 0)];
            assert_eq!(Arc::as_ptr(pasted), source.as_ptr());
            let decoded = decode(pasted).unwrap();
            assert_eq!(decoded.size, [4, 2]);
            assert!(decoded.pixels.iter().all(|pixel| *pixel == Color32::RED));
        }
        run(&mut app, "edit.clearClipboard", json!({}));
        assert!(app.session.clipboard.is_none() && app.session.format_painter.is_none());
        hidden_ticks(&ctx, &mut app);
        assert!(source.upgrade().is_some());
        assert_eq!(cache_state(&ctx), Some(initial), "pasted destination remains a legitimate owner after clear");
        close_active_document(&mut app);
        assert_source_dead(&source);
        hidden_ticks(&ctx, &mut app);
        assert_eq!(cache_state(&ctx), Some((0, 0, None)), "release after clipboard and destination owners are gone");
        Ok(())
    }

    #[test]
    fn dead_invalid_picture_entry_is_pruned_by_logic() {
        let ctx = egui::Context::default();
        let (mut app, source, _) = picture_app(Arc::new(CellPicture { data: vec![], mime: "image/png".into(), alt: "Invalid".into() }));
        paint_once(&ctx, &app);
        assert_eq!(cache_state(&ctx), Some((1, 0, None)));
        close_active_document(&mut app);
        assert_source_dead(&source);
        hidden_ticks(&ctx, &mut app);
        assert_eq!(cache_state(&ctx), Some((0, 0, None)), "zero-byte entries must be removed too");
    }

    #[test]
    fn logic_without_cell_pictures_does_not_create_a_cache() {
        let ctx = egui::Context::default();
        let mut session = Session::new();
        session.new_workbook();
        let mut app = SheetApp::new(session, Services::default());
        app.ui.autosave = false;
        assert!(cache_state(&ctx).is_none());
        for _ in 0..3 {
            hidden_ticks(&ctx, &mut app);
            assert!(cache_state(&ctx).is_none(), "no-image hidden logic must not allocate a cell cache");
        }
    }

    #[test]
    fn live_entry_and_cleanup_are_idempotent() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let (mut app, source, _) = picture_app(picture([255, 0, 0, 255])?);
        paint_once(&ctx, &app);
        let initial = cache_state(&ctx).unwrap();
        assert_eq!((initial.0, initial.1), (1, 32));
        assert!(initial.2.is_some());
        for _ in 0..3 {
            hidden_ticks(&ctx, &mut app);
            assert!(source.upgrade().is_some());
            assert_eq!(cache_state(&ctx), Some(initial));
        }
        close_active_document(&mut app);
        assert_source_dead(&source);
        for _ in 0..3 {
            hidden_ticks(&ctx, &mut app);
            assert_eq!(cache_state(&ctx), Some((0, 0, None)), "repeated cleanup must remain empty");
        }
        Ok(())
    }

    #[test]
    fn aspect_fit_and_bounded_content_cache() -> Result<(), Box<dyn std::error::Error>> {
        let cell = Rect::from_min_size(pos2(10.0, 20.0), vec2(100.0, 100.0));
        assert_eq!(fitted_rect(cell, [200, 100]), Some(Rect::from_min_size(pos2(10.0, 45.0), vec2(100.0, 50.0))));
        assert_eq!(fitted_rect(cell, [100, 200]), Some(Rect::from_min_size(pos2(35.0, 20.0), vec2(50.0, 100.0))));
        assert!(fitted_rect(Rect::NOTHING, [200, 100]).is_none());
        let ctx = egui::Context::default();
        let mut cache = Cache::default();
        let red = picture([255, 0, 0, 255])?;
        let blue = picture([0, 0, 255, 255])?;
        // Distinct content with identical dimensions must receive distinct handles.
        let red_id = cache.texture(&ctx, &red).ok_or("missing red texture")?.id();
        assert_eq!(cache.texture(&ctx, &red).ok_or("missing red texture")?.id(), red_id);
        assert_ne!(cache.texture(&ctx, &blue).ok_or("missing blue texture")?.id(), red_id);
        let payloads: Vec<_> = (0..133).map(|_| picture([0, 0, 255, 255])).collect::<Result<_, _>>()?;
        for payload in &payloads {
            cache.texture(&ctx, payload);
        }
        let ids: Vec<_> =
            payloads.iter().map(|p| cache.texture(&ctx, p).ok_or("missing visible texture").map(|t| t.id())).collect::<Result<_, _>>()?;
        for _ in 0..3 {
            let mut output = ctx.run_ui(Default::default(), |_| {
                for (payload, id) in payloads.iter().zip(&ids) {
                    assert_eq!(cache.texture(&ctx, payload).map(|t| t.id()), Some(*id));
                }
            });
            // This test observes handles without a renderer consuming their upload deltas.
            output.textures_delta.clear();
        }
        assert!(cache.entries.len() <= MAX_CACHE_ITEMS);
        assert!(cache.bytes <= MAX_CACHE_BYTES);
        assert!(decode(&CellPicture { data: vec![], mime: "image/png".into(), alt: String::new() }).is_none());
        assert_eq!(decode(&red).ok_or("decode failed")?.pixels.first(), Some(&Color32::RED));
        Ok(())
    }
}
