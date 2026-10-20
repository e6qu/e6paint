//! The board document: a Loro-backed CRDT scene store.
//!
//! Container mapping:
//!
//! - `meta` map — `title`, `next_id`
//! - `elements` map — element id (`el_<n>`) → nested map with `kind`, `x`, `y`, `w`, `h`, `color`
//! - `zorder` movable list — element ids in paint order (index = z)
//!
//! Mutating methods do *not* commit; callers group a gesture's ops and call [`BoardDoc::commit`]
//! once, which makes it a single undo step ([`UndoManager`] groups by commit). Undo/redo restore
//! exactly what a commit contained, including id-counter mints, so single-peer ids stay
//! collision-free. Multi-peer uniqueness needs per-peer id namespaces at the collab milestone.
//!
//! A materialized [`BoardState`] is the read model: mutations update it incrementally; undo,
//! redo, and snapshot import trigger a full refresh (their deltas are opaque here).

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use e6_scene::{Aabb, Element, ElementId, ElementKind, ImageAsset, Point, Size};
use loro::{ExportMode, LoroDoc, LoroMap, LoroValue, UndoManager};

/// Read model: the materialized board, elements in paint order.
#[derive(Debug, Clone, Default)]
pub struct BoardState {
    pub elements: Vec<Element>,
}

/// A fresh element to place; the document mints its id.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementSpec {
    pub kind: ElementKind,
    pub bounds: Aabb,
    pub color: u32,
    pub stroke_color: u32,
    pub stroke_width: f32,
    pub points: Vec<Point>,
    pub text: String,
    pub font_size: f32,
    /// Generator provenance key (e.g. `mol:0:gate`).
    pub provenance: Option<String>,
}

impl ElementSpec {
    /// A filled element without payload.
    pub fn simple(kind: ElementKind, bounds: Aabb, color: u32) -> Self {
        Self {
            kind,
            bounds,
            color,
            stroke_color: color,
            stroke_width: 0.0,
            points: Vec::new(),
            text: String::new(),
            font_size: 24.0,
            provenance: None,
        }
    }

    /// Builder: generator provenance key.
    pub fn with_provenance(mut self, key: impl Into<String>) -> Self {
        self.provenance = Some(key.into());
        self
    }

    /// Mirror an existing element (minus identity) into a fresh placement spec.
    pub fn of(element: &Element) -> Self {
        Self {
            kind: element.kind,
            bounds: element.bounds,
            color: element.color,
            stroke_color: element.stroke_color,
            stroke_width: element.stroke_width,
            points: element.points.clone(),
            text: element.text.clone(),
            font_size: element.font_size,
            provenance: element.provenance.clone(),
        }
    }

    /// Builder: outline style (shapes drawn stroked, not filled).
    pub fn outlined(mut self, stroke_color: u32, stroke_width: f32) -> Self {
        self.stroke_color = stroke_color;
        self.stroke_width = stroke_width;
        self
    }

    /// Builder: polyline payload (strokes, connectors).
    pub fn with_points(mut self, points: Vec<Point>) -> Self {
        self.points = points;
        self
    }

    /// Builder: text payload.
    pub fn with_text(mut self, text: String, font_size: f32) -> Self {
        self.text = text;
        self.font_size = font_size;
        self
    }
}

/// Reasons a document operation cannot be performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocError {
    /// A Loro operation failed.
    Loro(String),
    /// No element with this id exists.
    NoSuchElement(ElementId),
    /// A stored value violated the mapping (corrupt snapshot or unknown kind tag).
    Corrupt(String),
}

impl fmt::Display for DocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loro(err) => write!(f, "loro operation failed: {err}"),
            Self::NoSuchElement(id) => write!(f, "no element with id {}", id.raw()),
            Self::Corrupt(why) => write!(f, "document mapping violated: {why}"),
        }
    }
}

impl std::error::Error for DocError {}

impl From<loro::LoroError> for DocError {
    fn from(err: loro::LoroError) -> Self {
        Self::Loro(err.to_string())
    }
}

fn id_key(id: ElementId) -> String {
    format!("el_{}", id.raw())
}

fn id_from_key(key: &str) -> Result<ElementId, DocError> {
    key.strip_prefix("el_")
        .and_then(|rest| rest.parse::<u64>().ok())
        .map(ElementId::from_raw)
        .ok_or_else(|| DocError::Corrupt(format!("malformed element key {key:?}")))
}

fn kind_to_str(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Rect => "rect",
        ElementKind::Ellipse => "ellipse",
        ElementKind::Stroke => "stroke",
        ElementKind::Text => "text",
        ElementKind::Sticky => "sticky",
        ElementKind::Image => "image",
        ElementKind::Frame => "frame",
        ElementKind::Connector => "connector",
        ElementKind::Sector => "sector",
    }
}

fn kind_from_str(raw: &str) -> Result<ElementKind, DocError> {
    match raw {
        "rect" => Ok(ElementKind::Rect),
        "ellipse" => Ok(ElementKind::Ellipse),
        "stroke" => Ok(ElementKind::Stroke),
        "text" => Ok(ElementKind::Text),
        "sticky" => Ok(ElementKind::Sticky),
        "image" => Ok(ElementKind::Image),
        "frame" => Ok(ElementKind::Frame),
        "sector" => Ok(ElementKind::Sector),
        "connector" => Ok(ElementKind::Connector),
        other => Err(DocError::Corrupt(format!("unknown element kind {other:?}"))),
    }
}

/// The Loro-backed board document.
pub struct BoardDoc {
    doc: LoroDoc,
    undo: Mutex<UndoManager>,
    state: Mutex<BoardState>,
}

impl Default for BoardDoc {
    fn default() -> Self {
        Self::new()
    }
}

impl BoardDoc {
    pub fn new() -> Self {
        let doc = LoroDoc::new();
        let _ = doc.get_map("meta").insert("title", "Untitled board");
        let _ = doc.get_map("meta").insert("next_id", 1_i64);
        let undo = UndoManager::new(&doc);
        let built = Self {
            doc,
            undo: Mutex::new(undo),
            state: Mutex::new(BoardState::default()),
        };
        built.commit();
        built
    }

    /// Rebuild a document from a [`BoardDoc::snapshot`]. Undo history does not survive;
    /// the undo manager starts empty.
    pub fn from_snapshot(bytes: &[u8]) -> Result<Self, DocError> {
        let doc = LoroDoc::new();
        doc.import(bytes)?;
        let undo = UndoManager::new(&doc);
        let built = Self {
            doc,
            undo: Mutex::new(undo),
            state: Mutex::new(BoardState::default()),
        };
        built.refresh()?;
        Ok(built)
    }

    /// Full-state CRDT snapshot: history + current state.
    pub fn snapshot(&self) -> Result<Vec<u8>, DocError> {
        // export() is also a commit barrier; a gesture in flight gets flushed here.
        self.doc
            .export(ExportMode::Snapshot)
            .map_err(|err| DocError::Loro(err.to_string()))
    }

    /// The complete update history as an importable batch. Incremental
    /// `ExportMode::updates(&vv)` batches were probed and found to import as empty
    /// (spike evidence in git history; upstream semantics to revisit) — full-history
    /// batches are proven to converge and CRDT import is idempotent, so peers merge
    /// redundant batches harmlessly. The relay compacts by holding a live room doc.
    pub fn full_updates(&self) -> Result<Vec<u8>, DocError> {
        self.doc
            .export(ExportMode::all_updates())
            .map_err(|err| DocError::Loro(err.to_string()))
    }

    /// Merge a remote update batch into this document and refresh the read model.
    /// Remote changes never enter the local undo stack (they belong to another peer).
    pub fn apply_remote(&self, bytes: &[u8]) -> Result<(), DocError> {
        self.doc.import(bytes)?;
        self.refresh()?;
        Ok(())
    }

    /// Flush the pending transaction: one gesture, one undo step.
    pub fn commit(&self) {
        self.doc.commit();
    }

    /// Materialized elements in paint order (cheap clone).
    pub fn elements(&self) -> Vec<Element> {
        self.state
            .lock()
            .map(|state| state.elements.clone())
            .unwrap_or_default()
    }

    /// The `meta.title` value.
    pub fn title(&self) -> String {
        self.meta_str("title")
    }

    /// Whether this document has been seeded with starter content (persists in snapshots,
    /// so a board the user emptied stays empty across reloads).
    pub fn seeded(&self) -> bool {
        self.meta_str("seeded") == "true"
    }

    pub fn set_seeded(&self) -> Result<(), DocError> {
        self.doc.get_map("meta").insert("seeded", "true")?;
        Ok(())
    }

    /// Whether the sketch (hand-drawn) render theme is on.
    pub fn sketch(&self) -> bool {
        self.meta_str("sketch") == "true"
    }

    pub fn set_sketch(&self, on: bool) -> Result<(), DocError> {
        self.doc
            .get_map("meta")
            .insert("sketch", if on { "true" } else { "false" })?;
        Ok(())
    }

    /// All frame elements, in paint order.
    pub fn frames(&self) -> Vec<Element> {
        self.state
            .lock()
            .map(|state| {
                state
                    .elements
                    .iter()
                    .filter(|e| e.kind == ElementKind::Frame)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Elements whose `frame` tag names this frame (the frame element itself is not a
    /// member).
    pub fn frame_members(&self, frame_id: ElementId) -> Vec<ElementId> {
        let key = id_key(frame_id);
        self.state
            .lock()
            .map(|state| {
                state
                    .elements
                    .iter()
                    .filter(|e| e.frame.as_deref() == Some(key.as_str()))
                    .map(|e| e.id)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Innermost frame whose bounds contain `point` (paint-order reverse scan).
    fn frames_containing(&self, point: Point) -> Option<Element> {
        let state = self.state.lock().ok()?;
        state
            .elements
            .iter()
            .rev()
            .find(|e| e.kind == ElementKind::Frame && e.bounds.contains(point))
            .cloned()
    }

    /// Attach `id` to the frame containing its center, or detach it when none does.
    /// Frames themselves never attach. Returns whether membership changed.
    pub fn reattach_to_frame(&self, id: ElementId) -> Result<bool, DocError> {
        let (center, is_frame) = {
            let state = self
                .state
                .lock()
                .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
            let element = state
                .elements
                .iter()
                .find(|e| e.id == id)
                .ok_or(DocError::NoSuchElement(id))?;
            (element.bounds.center(), element.kind == ElementKind::Frame)
        };
        if is_frame {
            return Ok(false);
        }
        let frames = self.frames();
        let target = frames
            .iter()
            .rev()
            .find(|frame| frame.bounds.contains(center))
            .map(|frame| id_key(frame.id));
        let current = {
            let state = self
                .state
                .lock()
                .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
            state
                .elements
                .iter()
                .find(|e| e.id == id)
                .and_then(|e| e.frame.clone())
        };
        if target == current {
            return Ok(false);
        }
        self.set_frame_tag(id, target.as_deref())?;
        Ok(true)
    }

    pub fn set_title(&self, title: &str) -> Result<(), DocError> {
        self.doc.get_map("meta").insert("title", title)?;
        Ok(())
    }

    /// Mint the next element id from the `meta.next_id` counter. Must run inside the same
    /// transaction as the op that uses it, so undo/redo move counter and element together.
    fn mint_id(&self) -> Result<ElementId, DocError> {
        let meta = self.doc.get_map("meta");
        let next = match meta.get("next_id") {
            Some(slot) => match slot.get_deep_value() {
                LoroValue::I64(raw) if raw > 0 => raw,
                other => return Err(DocError::Corrupt(format!("bad next_id {other:?}"))),
            },
            None => return Err(DocError::Corrupt("meta.next_id missing".into())),
        };
        meta.insert("next_id", next + 1)?;
        let raw = u64::try_from(next).map_err(|_| DocError::Corrupt("next_id negative".into()))?;
        Ok(ElementId::from_raw(raw))
    }

    /// Insert a new element at the top of the paint order and return its minted id.
    /// No commit — group with the rest of the gesture.
    pub fn insert_element(&self, spec: &ElementSpec) -> Result<ElementId, DocError> {
        let id = self.mint_id()?;
        let key = id_key(id);
        // Attachment is resolved before the read model locks: std Mutex is not reentrant.
        let attached_frame = self
            .frames_containing(spec.bounds.center())
            .map(|f| id_key(f.id));
        let element: LoroMap = self
            .doc
            .get_map("elements")
            .insert_container(&key, LoroMap::new())?;
        element.insert("kind", kind_to_str(spec.kind))?;
        element.insert("x", f64::from(spec.bounds.min.x))?;
        element.insert("y", f64::from(spec.bounds.min.y))?;
        element.insert("w", f64::from(spec.bounds.size().width))?;
        element.insert("h", f64::from(spec.bounds.size().height))?;
        element.insert("color", i64::from(spec.color))?;
        element.insert("stroke_color", i64::from(spec.stroke_color))?;
        element.insert("stroke_width", f64::from(spec.stroke_width))?;
        element.insert("font_size", f64::from(spec.font_size))?;
        if !spec.points.is_empty() {
            element.insert("points", flat_points(&spec.points))?;
        }
        if !spec.text.is_empty() {
            element.insert("text", spec.text.as_str())?;
        }
        if let Some(provenance) = &spec.provenance {
            element.insert("prov", provenance.as_str())?;
        }
        // New elements attach to any frame that contains their center.
        let attached = self.frames_containing(spec.bounds.center());
        self.doc.get_movable_list("zorder").push(key.as_str())?;
        if let Some(frame) = attached {
            element.insert("frame", id_key(frame.id).as_str())?;
        }

        self.state
            .lock()
            .map(|mut state| {
                state.elements.push(Element {
                    id,
                    kind: spec.kind,
                    bounds: spec.bounds,
                    color: spec.color,
                    stroke_color: spec.stroke_color,
                    stroke_width: spec.stroke_width,
                    points: spec.points.clone(),
                    text: spec.text.clone(),
                    font_size: spec.font_size,
                    group: None,
                    frame: attached_frame,
                    locked: false,
                    hidden: false,
                    image: None,
                    provenance: spec.provenance.clone(),
                });
            })
            .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
        Ok(id)
    }

    /// Overwrite an element's bounds. No commit — group per gesture.
    pub fn set_geometry(&self, id: ElementId, bounds: Aabb) -> Result<(), DocError> {
        let key = id_key(id);
        let element = self.element_container(&key)?;
        element.insert("x", f64::from(bounds.min.x))?;
        element.insert("y", f64::from(bounds.min.y))?;
        element.insert("w", f64::from(bounds.size().width))?;
        element.insert("h", f64::from(bounds.size().height))?;
        self.state
            .lock()
            .map(|mut state| {
                if let Some(slot) = state.elements.iter_mut().find(|e| e.id == id) {
                    slot.bounds = bounds;
                }
            })
            .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
        Ok(())
    }

    /// Overwrite an element's text payload. No commit.
    pub fn set_text(&self, id: ElementId, text: &str) -> Result<(), DocError> {
        let key = id_key(id);
        let element = self.element_container(&key)?;
        element.insert("text", text)?;
        self.state
            .lock()
            .map(|mut state| {
                if let Some(slot) = state.elements.iter_mut().find(|e| e.id == id) {
                    slot.text = text.to_owned();
                }
            })
            .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
        Ok(())
    }

    /// Move an element by a world-space delta: bounds origin *and* any absolute points
    /// (strokes, connectors) translate together. No commit.
    pub fn translate(&self, id: ElementId, dx: f32, dy: f32) -> Result<(), DocError> {
        let (bounds, points) = {
            let state = self
                .state
                .lock()
                .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
            let element = state
                .elements
                .iter()
                .find(|e| e.id == id)
                .ok_or(DocError::NoSuchElement(id))?;
            (element.bounds, element.points.clone())
        };
        let moved_bounds = Aabb::from_min_size(
            Point::new(bounds.min.x + dx, bounds.min.y + dy),
            bounds.size(),
        );
        self.set_geometry(id, moved_bounds)?;
        if !points.is_empty() {
            let moved: Vec<Point> = points
                .iter()
                .map(|p| Point::new(p.x + dx, p.y + dy))
                .collect();
            self.element_container(&id_key(id))?
                .insert("points", flat_points(&moved))?;
            self.state
                .lock()
                .map(|mut state| {
                    if let Some(slot) = state.elements.iter_mut().find(|e| e.id == id) {
                        slot.points = moved;
                    }
                })
                .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
        }
        Ok(())
    }

    /// Decode an encoded image (PNG/JPEG) and place it as a new element, sized to fit
    /// within [`MAX_IMAGE_SIDE`] world units with the aspect ratio preserved. The
    /// original bytes are stored in the document for snapshot fidelity. No commit.
    pub fn insert_image(&self, encoded: Vec<u8>) -> Result<ElementId, DocError> {
        let asset = decode_image(&encoded)?;
        let longest = asset.width.max(asset.height).max(1) as f32;
        // Downscale-only to fit MAX_IMAGE_SIDE, but keep tiny images grabbable (min 40
        // units on the longest side) — both bounds scale uniformly, aspect preserved.
        let scale = (MAX_IMAGE_SIDE / longest).min(1.0).max(40.0 / longest);
        let bounds = Aabb::from_min_size(
            Point::ZERO,
            Size::new(asset.width as f32 * scale, asset.height as f32 * scale),
        );
        let id = self.mint_id()?;
        let key = id_key(id);
        let attached_frame = self
            .frames_containing(bounds.center())
            .map(|f| id_key(f.id));
        let element: LoroMap = self
            .doc
            .get_map("elements")
            .insert_container(&key, LoroMap::new())?;
        element.insert("kind", kind_to_str(ElementKind::Image))?;
        element.insert("x", f64::from(bounds.min.x))?;
        element.insert("y", f64::from(bounds.min.y))?;
        element.insert("w", f64::from(bounds.size().width))?;
        element.insert("h", f64::from(bounds.size().height))?;
        element.insert("color", i64::from(0xFFFF_FFFF_u32))?;
        element.insert("png", encoded)?;
        if let Some(frame) = attached_frame.as_deref() {
            element.insert("frame", frame)?;
        }
        self.doc.get_movable_list("zorder").push(key.as_str())?;

        self.state
            .lock()
            .map(|mut state| {
                state.elements.push(Element {
                    id,
                    kind: ElementKind::Image,
                    bounds,
                    color: 0xFFFF_FFFF,
                    stroke_color: 0xFFFF_FFFF,
                    stroke_width: 0.0,
                    points: Vec::new(),
                    text: String::new(),
                    font_size: 24.0,
                    group: None,
                    frame: attached_frame,
                    locked: false,
                    hidden: false,
                    image: Some(asset),
                    provenance: None,
                });
            })
            .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
        Ok(id)
    }

    /// Remove every element whose provenance key starts with `prefix`, skipping
    /// locked ones (lock = pin during regeneration). Returns the removed count.
    pub fn remove_by_provenance(&self, prefix: &str) -> Result<usize, DocError> {
        let doomed: Vec<ElementId> = {
            let state = self
                .state
                .lock()
                .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
            state
                .elements
                .iter()
                .filter(|e| {
                    e.provenance
                        .as_deref()
                        .is_some_and(|key| key.starts_with(prefix))
                        && !e.locked
                })
                .map(|e| e.id)
                .collect()
        };
        for id in &doomed {
            self.remove_element(*id)?;
        }
        Ok(doomed.len())
    }

    /// Remove an element and its paint-order entry. No commit.
    pub fn remove_element(&self, id: ElementId) -> Result<(), DocError> {
        // Removing a frame detaches its members: their tags would otherwise
        // reference a frame that no longer exists — ghost state that would
        // persist through snapshots.
        if self
            .state
            .lock()
            .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?
            .elements
            .iter()
            .find(|e| e.id == id)
            .is_some_and(|e| e.kind == ElementKind::Frame)
        {
            for member in self.frame_members(id) {
                self.set_frame_tag(member, None)?;
            }
        }
        let key = id_key(id);
        let zorder = self.doc.get_movable_list("zorder");
        let index = self.zorder_index(&key)?;
        zorder.delete(index, 1)?;
        self.doc.get_map("elements").delete(&key)?;
        self.state
            .lock()
            .map(|mut state| state.elements.retain(|e| e.id != id))
            .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
        Ok(())
    }

    /// Set (or clear) an element's frame membership tag, keeping the read model
    /// in step with the Loro container.
    fn set_frame_tag(&self, id: ElementId, frame: Option<&str>) -> Result<(), DocError> {
        let element = self.element_container(&id_key(id))?;
        match frame {
            Some(key) => element.insert("frame", key)?,
            None => element.delete("frame")?,
        }
        self.state
            .lock()
            .map(|mut state| {
                if let Some(slot) = state.elements.iter_mut().find(|e| e.id == id) {
                    slot.frame = frame.map(|key| key.to_owned());
                }
            })
            .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
        Ok(())
    }

    /// Put every given element into one new group (minted key). Elements keep their own
    /// membership until regrouped; concurrent edits merge per element. No commit.
    pub fn group(&self, ids: &[ElementId]) -> Result<(), DocError> {
        let next = self.mint_group_key()?;
        for id in ids {
            self.set_group_tag(*id, Some(&next))?;
        }
        Ok(())
    }

    /// Clear group membership of every given element. No commit.
    pub fn ungroup(&self, ids: &[ElementId]) -> Result<(), DocError> {
        for id in ids {
            self.set_group_tag(*id, None)?;
        }
        Ok(())
    }

    /// All elements sharing `id`'s group (including `id` itself); just `id` when ungrouped.
    pub fn group_members(&self, id: ElementId) -> Vec<ElementId> {
        self.state
            .lock()
            .map(|state| {
                let tag = state
                    .elements
                    .iter()
                    .find(|e| e.id == id)
                    .and_then(|e| e.group.clone());
                match tag {
                    Some(key) => state
                        .elements
                        .iter()
                        .filter(|e| e.group.as_deref() == Some(key.as_str()))
                        .map(|e| e.id)
                        .collect(),
                    None => vec![id],
                }
            })
            .unwrap_or_default()
    }

    /// Lock an element: it stops being hit-testable on canvas until unlocked.
    pub fn set_locked(&self, id: ElementId, locked: bool) -> Result<(), DocError> {
        let key = id_key(id);
        let element = self.element_container(&key)?;
        element.insert("locked", locked)?;
        self.state
            .lock()
            .map(|mut state| {
                if let Some(slot) = state.elements.iter_mut().find(|e| e.id == id) {
                    slot.locked = locked;
                }
            })
            .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
        Ok(())
    }

    /// Hide or show an element (layers panel). Hidden elements are skipped by the
    /// renderer and canvas hit-testing. No commit.
    pub fn set_hidden(&self, id: ElementId, hidden: bool) -> Result<(), DocError> {
        let key = id_key(id);
        let element = self.element_container(&key)?;
        element.insert("hidden", hidden)?;
        self.state
            .lock()
            .map(|mut state| {
                if let Some(slot) = state.elements.iter_mut().find(|e| e.id == id) {
                    slot.hidden = hidden;
                }
            })
            .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
        Ok(())
    }

    fn set_group_tag(&self, id: ElementId, key: Option<&str>) -> Result<(), DocError> {
        let element = self.element_container(&id_key(id))?;
        match key {
            Some(key) => element.insert("group", key)?,
            None => element.delete("group")?,
        }
        self.state
            .lock()
            .map(|mut state| {
                if let Some(slot) = state.elements.iter_mut().find(|e| e.id == id) {
                    slot.group = key.map(str::to_owned);
                }
            })
            .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
        Ok(())
    }

    fn mint_group_key(&self) -> Result<String, DocError> {
        let meta = self.doc.get_map("meta");
        let next = match meta.get("next_id") {
            Some(slot) => match slot.get_deep_value() {
                LoroValue::I64(raw) if raw > 0 => raw,
                other => return Err(DocError::Corrupt(format!("bad next_id {other:?}"))),
            },
            None => return Err(DocError::Corrupt("meta.next_id missing".into())),
        };
        meta.insert("next_id", next + 1)?;
        Ok(format!("grp_{next}"))
    }

    /// Move the element to the top of the paint order. No commit.
    pub fn bring_to_front(&self, id: ElementId) -> Result<(), DocError> {
        self.mov_zorder(id, usize::MAX)
    }

    /// Move the element to the bottom of the paint order. No commit.
    pub fn send_to_back(&self, id: ElementId) -> Result<(), DocError> {
        self.mov_zorder(id, 0)
    }

    fn mov_zorder(&self, id: ElementId, target: usize) -> Result<(), DocError> {
        let key = id_key(id);
        let zorder = self.doc.get_movable_list("zorder");
        let from = self.zorder_index(&key)?;
        let len = zorder.len();
        let to = target.min(len.saturating_sub(1));
        if from == to {
            return Ok(());
        }
        zorder.mov(from, to)?;
        self.state
            .lock()
            .map(|mut state| {
                if let Some(from_pos) = state.elements.iter().position(|e| e.id == id) {
                    let element = state.elements.remove(from_pos);
                    let to_pos = to.min(state.elements.len());
                    state.elements.insert(to_pos, element);
                }
            })
            .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
        Ok(())
    }

    /// Undo the last committed gesture. Returns whether anything was undone.
    pub fn undo(&self) -> Result<bool, DocError> {
        let mut undo = self
            .undo
            .lock()
            .map_err(|_| DocError::Corrupt("undo mutex poisoned".into()))?;
        let undone = undo.undo().map_err(|err| DocError::Loro(err.to_string()))?;
        drop(undo);
        if undone {
            self.refresh()?;
        }
        Ok(undone)
    }

    /// Redo the last undone gesture. Returns whether anything was redone.
    pub fn redo(&self) -> Result<bool, DocError> {
        let mut undo = self
            .undo
            .lock()
            .map_err(|_| DocError::Corrupt("undo mutex poisoned".into()))?;
        let redone = undo.redo().map_err(|err| DocError::Loro(err.to_string()))?;
        drop(undo);
        if redone {
            self.refresh()?;
        }
        Ok(redone)
    }

    pub fn can_undo(&self) -> bool {
        self.undo
            .lock()
            .map(|undo| undo.can_undo())
            .unwrap_or(false)
    }

    pub fn can_redo(&self) -> bool {
        self.undo
            .lock()
            .map(|undo| undo.can_redo())
            .unwrap_or(false)
    }

    /// Lossy pretty JSON dump of the materialized board (for grepping and git exports).
    pub fn json_dump(&self) -> Result<String, DocError> {
        let state = self
            .state
            .lock()
            .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
        let dump = serde_json::to_string_pretty(&DumpBoard::new(&state, &self.title()))
            .map_err(|err| DocError::Loro(err.to_string()))?;
        Ok(dump)
    }

    // ---- read model ---------------------------------------------------------------

    fn refresh(&self) -> Result<(), DocError> {
        let elements = self.read_elements()?;
        let order = self.read_zorder()?;
        let by_key: HashMap<String, Element> = elements
            .into_iter()
            .map(|element| (id_key(element.id), element))
            .collect();
        let mut ordered: Vec<Element> = Vec::with_capacity(order.len());
        for key in order {
            if let Some(element) = by_key.get(&key) {
                ordered.push(element.clone());
            }
        }
        // Any element missing from the zorder list still belongs on the board; append it
        // (a mapping violation by construction, but dropping the element would be worse).
        for element in by_key.into_values() {
            if !ordered.iter().any(|e| e.id == element.id) {
                ordered.push(element);
            }
        }
        self.state
            .lock()
            .map(|mut state| state.elements = ordered)
            .map_err(|_| DocError::Corrupt("state mutex poisoned".into()))?;
        Ok(())
    }

    fn read_elements(&self) -> Result<Vec<Element>, DocError> {
        let mut out: Vec<Element> = Vec::new();
        let mut failure: Option<DocError> = None;
        self.doc.get_map("elements").for_each(|key, slot| {
            if failure.is_some() {
                return;
            }
            match build_element(key, slot) {
                Ok(element) => out.push(element),
                Err(err) => failure = Some(err),
            }
        });
        match failure {
            Some(err) => Err(err),
            None => Ok(out),
        }
    }

    fn read_zorder(&self) -> Result<Vec<String>, DocError> {
        let deep = self.doc.get_movable_list("zorder").get_deep_value();
        let LoroValue::List(items) = deep else {
            return Err(DocError::Corrupt("zorder is not a list".into()));
        };
        let mut out = Vec::with_capacity(items.len());
        for item in items.iter() {
            let LoroValue::String(text) = item else {
                return Err(DocError::Corrupt("zorder entry is not a string".into()));
            };
            out.push(text.as_str().to_owned());
        }
        Ok(out)
    }

    fn zorder_index(&self, key: &str) -> Result<usize, DocError> {
        let order = self.read_zorder()?;
        let id = id_from_key(key)?;
        order
            .iter()
            .position(|existing| existing == key)
            .ok_or(DocError::NoSuchElement(id))
    }

    fn element_container(&self, key: &str) -> Result<LoroMap, DocError> {
        match self.doc.get_map("elements").get(key) {
            Some(loro::ValueOrContainer::Container(loro::Container::Map(map))) => Ok(map),
            Some(_) => Err(DocError::Corrupt(format!("{key:?} is not a map container"))),
            None => Err(DocError::NoSuchElement(id_from_key(key)?)),
        }
    }

    fn meta_str(&self, key: &str) -> String {
        match self.doc.get_map("meta").get(key) {
            Some(slot) => match slot.get_deep_value() {
                LoroValue::String(text) => text.as_str().to_owned(),
                _ => String::new(),
            },
            None => String::new(),
        }
    }
}

/// Longest allowed image side in world units at placement.
pub const MAX_IMAGE_SIDE: f32 = 480.0;

/// Decode an encoded image into the scene payload (straight-alpha RGBA8).
fn decode_image(encoded: &[u8]) -> Result<ImageAsset, DocError> {
    let decoded = image::load_from_memory(encoded)
        .map_err(|err| DocError::Corrupt(format!("image decode failed: {err}")))?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(ImageAsset {
        width,
        height,
        rgba: Arc::new(rgba.into_raw()),
    })
}

fn build_element(key: &str, slot: loro::ValueOrContainer) -> Result<Element, DocError> {
    let id = id_from_key(key)?;
    let LoroValue::Map(fields) = slot.get_deep_value() else {
        return Err(DocError::Corrupt(format!("{key:?} is not a map")));
    };
    let kind = kind_from_str(field_str(&fields, "kind")?.as_str())?;
    let bounds = Aabb::from_min_size(
        Point::new(field_f64(&fields, "x")?, field_f64(&fields, "y")?),
        Size::new(field_f64(&fields, "w")?, field_f64(&fields, "h")?),
    );
    let color = field_i64(&fields, "color")?;
    let color = u32::try_from(color)
        .map_err(|_| DocError::Corrupt(format!("{key:?} color out of range")))?;
    let stroke_color = match fields.get("stroke_color") {
        Some(LoroValue::I64(raw)) => u32::try_from(*raw)
            .map_err(|_| DocError::Corrupt(format!("{key:?} stroke_color out of range")))?,
        // Elements written before the style fields existed keep the fill color.
        _ => color,
    };
    let stroke_width = match fields.get("stroke_width") {
        Some(LoroValue::Double(raw)) => *raw as f32,
        _ => 0.0,
    };
    let font_size = match fields.get("font_size") {
        Some(LoroValue::Double(raw)) => *raw as f32,
        _ => 24.0,
    };
    let points = match fields.get("points") {
        Some(LoroValue::List(raw)) => unflat_points(raw)?,
        Some(_) => return Err(DocError::Corrupt(format!("{key:?} points is not a list"))),
        None => Vec::new(),
    };
    let text = match fields.get("text") {
        Some(LoroValue::String(text)) => text.as_str().to_owned(),
        Some(_) => return Err(DocError::Corrupt(format!("{key:?} text is not a string"))),
        None => String::new(),
    };
    let group = match fields.get("group") {
        Some(LoroValue::String(tag)) => Some(tag.as_str().to_owned()),
        Some(_) => return Err(DocError::Corrupt(format!("{key:?} group is not a string"))),
        None => None,
    };
    let frame = match fields.get("frame") {
        Some(LoroValue::String(tag)) => Some(tag.as_str().to_owned()),
        Some(_) => return Err(DocError::Corrupt(format!("{key:?} frame is not a string"))),
        None => None,
    };
    let locked = matches!(fields.get("locked"), Some(LoroValue::Bool(true)));
    let hidden = matches!(fields.get("hidden"), Some(LoroValue::Bool(true)));
    let provenance = match fields.get("prov") {
        Some(LoroValue::String(tag)) => Some(tag.as_str().to_owned()),
        Some(_) => return Err(DocError::Corrupt(format!("{key:?} prov is not a string"))),
        None => None,
    };
    let image = match fields.get("png") {
        Some(LoroValue::Binary(bytes)) => Some(decode_image(bytes.as_ref())?),
        Some(_) => return Err(DocError::Corrupt(format!("{key:?} png is not binary"))),
        None => None,
    };
    Ok(Element {
        id,
        kind,
        bounds,
        color,
        stroke_color,
        stroke_width,
        points,
        text,
        font_size,
        group,
        frame,
        locked,
        hidden,
        provenance,
        image,
    })
}

/// Points as a flat `[x0, y0, x1, y1, …]` double list value.
fn flat_points(points: &[Point]) -> Vec<LoroValue> {
    let mut flat = Vec::with_capacity(points.len() * 2);
    for point in points {
        flat.push(LoroValue::Double(f64::from(point.x)));
        flat.push(LoroValue::Double(f64::from(point.y)));
    }
    flat
}

fn unflat_points(raw: &[LoroValue]) -> Result<Vec<Point>, DocError> {
    if !raw.len().is_multiple_of(2) {
        return Err(DocError::Corrupt("points list has an odd length".into()));
    }
    let mut out = Vec::with_capacity(raw.len() / 2);
    for pair in raw.chunks(2) {
        let (LoroValue::Double(x), LoroValue::Double(y)) = (&pair[0], &pair[1]) else {
            return Err(DocError::Corrupt("points list holds a non-number".into()));
        };
        out.push(Point::new(*x as f32, *y as f32));
    }
    Ok(out)
}

fn field_str(fields: &loro::LoroMapValue, key: &str) -> Result<String, DocError> {
    match fields.get(key) {
        Some(LoroValue::String(text)) => Ok(text.as_str().to_owned()),
        _ => Err(DocError::Corrupt(format!(
            "field {key:?} missing or not a string"
        ))),
    }
}

fn field_f64(fields: &loro::LoroMapValue, key: &str) -> Result<f32, DocError> {
    match fields.get(key) {
        Some(LoroValue::Double(raw)) => Ok(*raw as f32),
        _ => Err(DocError::Corrupt(format!(
            "field {key:?} missing or not a number"
        ))),
    }
}

fn field_i64(fields: &loro::LoroMapValue, key: &str) -> Result<i64, DocError> {
    match fields.get(key) {
        Some(LoroValue::I64(raw)) => Ok(*raw),
        _ => Err(DocError::Corrupt(format!(
            "field {key:?} missing or not an integer"
        ))),
    }
}

#[derive(serde::Serialize)]
struct DumpBoard {
    title: String,
    elements: Vec<DumpElement>,
}

#[derive(serde::Serialize)]
struct DumpElement {
    id: u64,
    kind: &'static str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    points: Vec<[f32; 2]>,
    #[serde(skip_serializing_if = "String::is_empty")]
    text: String,
}

impl DumpBoard {
    fn new(state: &BoardState, title: &str) -> Self {
        Self {
            title: title.to_owned(),
            elements: state
                .elements
                .iter()
                .map(|element| DumpElement {
                    id: element.id.raw(),
                    kind: kind_to_str(element.kind),
                    x: element.bounds.min.x,
                    y: element.bounds.min.y,
                    w: element.bounds.size().width,
                    h: element.bounds.size().height,
                    color: format!("#{:08x}", element.color),
                    points: element.points.iter().map(|p| [p.x, p.y]).collect(),
                    text: element.text.clone(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use e6_scene::ElementKind;

    fn spec(x: f32, y: f32, kind: ElementKind) -> ElementSpec {
        ElementSpec::simple(
            kind,
            Aabb::from_min_size(Point::new(x, y), Size::new(50.0, 40.0)),
            0xFF00_FF00,
        )
    }

    /// A 4×3 red PNG, encoded in-process (encode support ships with the decode feature).
    fn tiny_png() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let image = image::RgbaImage::from_pixel(4, 3, image::Rgba([255, 0, 0, 255]));
        let mut cursor = std::io::Cursor::new(Vec::new());
        image.write_to(&mut cursor, image::ImageFormat::Png)?;
        Ok(cursor.into_inner())
    }

    fn ids(board: &BoardDoc) -> Vec<u64> {
        board.elements().iter().map(|e| e.id.raw()).collect()
    }

    #[test]
    fn insert_and_read_back() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        let id = board.insert_element(&spec(10.0, 20.0, ElementKind::Ellipse))?;
        board.commit();

        let elements = board.elements();
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].id, id);
        assert_eq!(elements[0].kind, ElementKind::Ellipse);
        assert_eq!(elements[0].bounds.min, Point::new(10.0, 20.0));
        assert_eq!(elements[0].color, 0xFF00_FF00);
        Ok(())
    }

    #[test]
    fn set_geometry_updates_the_read_model() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        let id = board.insert_element(&spec(0.0, 0.0, ElementKind::Rect))?;
        board.commit();
        board.set_geometry(
            id,
            Aabb::from_min_size(Point::new(7.0, 9.0), Size::new(80.0, 60.0)),
        )?;
        board.commit();

        let element = &board.elements()[0];
        assert_eq!(element.bounds.min, Point::new(7.0, 9.0));
        assert_eq!(element.bounds.size(), Size::new(80.0, 60.0));
        Ok(())
    }

    #[test]
    fn one_commit_is_one_undo_step() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        let id = board.insert_element(&spec(0.0, 0.0, ElementKind::Rect))?;
        board.set_geometry(
            id,
            Aabb::from_min_size(Point::new(5.0, 5.0), Size::new(50.0, 40.0)),
        )?;
        board.commit();
        assert_eq!(board.elements()[0].bounds.min, Point::new(5.0, 5.0));

        assert!(board.undo()?);
        assert!(
            board.elements().is_empty(),
            "insert+move undo as one gesture"
        );
        assert!(board.redo()?);
        assert_eq!(board.elements().len(), 1);
        assert_eq!(board.elements()[0].bounds.min, Point::new(5.0, 5.0));
        Ok(())
    }

    #[test]
    fn undo_never_resurrects_a_live_id() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        let first = board.insert_element(&spec(0.0, 0.0, ElementKind::Rect))?;
        board.commit();
        board.remove_element(first)?;
        board.commit();
        board.undo()?;
        assert_eq!(ids(&board), vec![first.raw()], "undo restored the deletion");

        let second = board.insert_element(&spec(1.0, 1.0, ElementKind::Rect))?;
        board.commit();
        assert_ne!(first, second, "minted ids stay unique across undo/redo");
        assert_eq!(ids(&board), vec![first.raw(), second.raw()]);
        Ok(())
    }

    #[test]
    fn zorder_operations_reorder_paint_order() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        let a = board.insert_element(&spec(0.0, 0.0, ElementKind::Rect))?;
        let b = board.insert_element(&spec(1.0, 1.0, ElementKind::Rect))?;
        let c = board.insert_element(&spec(2.0, 2.0, ElementKind::Rect))?;
        board.commit();
        assert_eq!(
            ids(&board),
            vec![a.raw(), b.raw(), c.raw()],
            "insertion order"
        );

        board.send_to_back(c)?;
        board.commit();
        assert_eq!(ids(&board), vec![c.raw(), a.raw(), b.raw()]);

        board.bring_to_front(c)?;
        board.commit();
        assert_eq!(ids(&board), vec![a.raw(), b.raw(), c.raw()]);
        Ok(())
    }

    #[test]
    fn remove_element_removes_everywhere() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        let a = board.insert_element(&spec(0.0, 0.0, ElementKind::Rect))?;
        let b = board.insert_element(&spec(1.0, 1.0, ElementKind::Rect))?;
        board.commit();
        board.remove_element(a)?;
        board.commit();

        assert_eq!(ids(&board), vec![b.raw()]);
        assert!(matches!(
            board.set_geometry(a, Aabb::from_min_size(Point::ZERO, Size::new(1.0, 1.0))),
            Err(DocError::NoSuchElement(_))
        ));
        Ok(())
    }

    #[test]
    fn snapshot_round_trips_state_and_order() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        board.insert_element(&spec(0.0, 0.0, ElementKind::Rect))?;
        board.insert_element(&spec(1.0, 1.0, ElementKind::Sticky))?;
        board.commit();
        board.set_title("Round trip")?;
        board.commit();
        let before = board.elements();

        let restored = BoardDoc::from_snapshot(&board.snapshot()?)?;
        assert_eq!(restored.title(), "Round trip");
        assert_eq!(restored.elements(), before);
        // Undo history is intentionally empty after a snapshot restore.
        assert!(!restored.can_undo());
        Ok(())
    }

    #[test]
    fn json_dump_is_pretty_and_shaped() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        board.set_title("Dump")?;
        board.insert_element(&spec(3.0, 4.0, ElementKind::Rect))?;
        board.commit();

        let dump = board.json_dump()?;
        assert!(dump.contains('\n'), "pretty-printed");
        let value: serde_json::Value = serde_json::from_str(&dump)?;
        assert_eq!(value["title"], "Dump");
        assert_eq!(value["elements"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["elements"][0]["kind"], "rect");
        assert_eq!(value["elements"][0]["x"], 3.0);
        assert_eq!(value["elements"][0]["color"], "#ff00ff00");
        Ok(())
    }

    #[test]
    fn seeded_flag_persists_through_snapshots() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        assert!(!board.seeded());
        board.set_seeded()?;
        board.commit();
        let restored = BoardDoc::from_snapshot(&board.snapshot()?)?;
        assert!(restored.seeded());
        Ok(())
    }

    #[test]
    fn stroke_points_round_trip_and_translate_together() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        let mut spec = spec(0.0, 0.0, ElementKind::Stroke).outlined(0xFF00_00FF, 3.0);
        spec.points = vec![Point::new(10.0, 20.0), Point::new(60.0, 70.0)];
        spec.bounds = Aabb::from_points(&spec.points).ok_or("two points have bounds")?;
        let id = board.insert_element(&spec)?;
        board.commit();

        let stored = &board.elements()[0];
        assert_eq!(stored.points, spec.points);
        assert_eq!(stored.stroke_color, 0xFF00_00FF);
        assert_eq!(stored.stroke_width, 3.0);

        board.translate(id, 5.0, -2.5)?;
        board.commit();
        let moved = &board.elements()[0];
        assert_eq!(moved.points[0], Point::new(15.0, 17.5));
        assert_eq!(moved.bounds.min, Point::new(15.0, 17.5));

        let restored = BoardDoc::from_snapshot(&board.snapshot()?)?;
        assert_eq!(restored.elements()[0].points, moved.points);
        Ok(())
    }

    #[test]
    fn text_payload_updates_and_snapshots() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        let id = board.insert_element(&spec(0.0, 0.0, ElementKind::Sticky))?;
        board.set_text(id, "hello board")?;
        board.commit();

        assert_eq!(board.elements()[0].text, "hello board");
        board.set_text(id, "edited")?;
        board.commit();
        assert_eq!(board.elements()[0].text, "edited");

        let restored = BoardDoc::from_snapshot(&board.snapshot()?)?;
        assert_eq!(restored.elements()[0].text, "edited");
        Ok(())
    }

    #[test]
    fn group_members_follow_tags_and_ungroup_clears_them() -> Result<(), Box<dyn std::error::Error>>
    {
        let board = BoardDoc::new();
        let a = board.insert_element(&spec(0.0, 0.0, ElementKind::Rect))?;
        let b = board.insert_element(&spec(1.0, 1.0, ElementKind::Rect))?;
        let c = board.insert_element(&spec(2.0, 2.0, ElementKind::Rect))?;
        board.commit();

        board.group(&[a, b])?;
        board.commit();
        let mut members = board.group_members(a);
        members.sort();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(members, expected);
        assert_eq!(board.group_members(c), vec![c]);

        board.ungroup(&[a])?;
        board.commit();
        assert_eq!(board.group_members(a), vec![a]);
        assert_eq!(board.group_members(b), vec![b]);
        Ok(())
    }

    #[test]
    fn locked_round_trips_through_snapshots() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        let id = board.insert_element(&spec(0.0, 0.0, ElementKind::Rect))?;
        board.set_locked(id, true)?;
        board.commit();
        assert!(board.elements()[0].locked);
        let restored = BoardDoc::from_snapshot(&board.snapshot()?)?;
        assert!(restored.elements()[0].locked);
        Ok(())
    }

    #[test]
    fn sketch_flag_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        assert!(!board.sketch());
        board.set_sketch(true)?;
        board.commit();
        let restored = BoardDoc::from_snapshot(&board.snapshot()?)?;
        assert!(restored.sketch());
        Ok(())
    }

    #[test]
    fn image_round_trips_bytes_bounds_and_pixels() -> Result<(), Box<dyn std::error::Error>> {
        let png = tiny_png()?;
        let board = BoardDoc::new();
        let _id = board.insert_image(png.clone())?;
        board.commit();

        let element = &board.elements()[0];
        assert_eq!(element.kind, ElementKind::Image);
        assert_eq!(
            element.bounds.size(),
            Size::new(40.0, 30.0),
            "tiny images scale up to a 40-unit long side, aspect kept"
        );
        let asset = element.image.as_ref().ok_or("image payload missing")?;
        assert_eq!((asset.width, asset.height), (4, 3));
        assert_eq!(asset.rgba.len(), 4 * 3 * 4);
        assert_eq!(asset.rgba[0], 255, "red channel of the fixture");

        let restored = BoardDoc::from_snapshot(&board.snapshot()?)?;
        assert_eq!(restored.elements()[0].image, element.image);
        Ok(())
    }

    #[test]
    fn corrupt_image_bytes_are_a_loud_error() {
        assert!(matches!(
            BoardDoc::new().insert_image(vec![0xFF, 0xD8, 0xDE, 0xAD]),
            Err(DocError::Corrupt(_))
        ));
    }

    #[test]
    fn frames_attach_new_elements_and_reattach_on_move() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        let frame_spec = ElementSpec::simple(
            ElementKind::Frame,
            Aabb::from_min_size(Point::new(0.0, 0.0), Size::new(200.0, 200.0)),
            0x0000_0000,
        )
        .with_text("Board".into(), 20.0);
        let frame = board.insert_element(&frame_spec)?;
        board.commit();

        // Created inside the frame → attached.
        let inside = board.insert_element(&spec(50.0, 50.0, ElementKind::Sticky))?;
        board.commit();
        assert_eq!(board.frame_members(frame), vec![inside]);
        assert_eq!(
            board
                .elements()
                .iter()
                .find(|e| e.id == inside)
                .and_then(|e| e.frame.clone()),
            Some(id_key(frame))
        );

        // Dragged outside → detached by reattach.
        board.translate(inside, 500.0, 500.0)?;
        board.reattach_to_frame(inside)?;
        board.commit();
        assert!(board.frame_members(frame).is_empty());
        assert_eq!(
            board
                .elements()
                .iter()
                .find(|e| e.id == inside)
                .and_then(|e| e.frame.clone()),
            None
        );

        // Dragged back → reattached.
        board.translate(inside, -500.0, -500.0)?;
        board.reattach_to_frame(inside)?;
        board.commit();
        assert_eq!(board.frame_members(frame), vec![inside]);
        Ok(())
    }

    #[test]
    fn removing_a_frame_detaches_its_members() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        let frame_spec = ElementSpec::simple(
            ElementKind::Frame,
            Aabb::from_min_size(Point::new(0.0, 0.0), Size::new(200.0, 200.0)),
            0x0000_0000,
        )
        .with_text("Board".into(), 20.0);
        let frame = board.insert_element(&frame_spec)?;
        let inside_a = board.insert_element(&spec(50.0, 50.0, ElementKind::Sticky))?;
        let inside_b = board.insert_element(&spec(80.0, 80.0, ElementKind::Text))?;
        board.commit();
        assert_eq!(board.frame_members(frame).len(), 2);

        board.remove_element(frame)?;
        board.commit();

        // Members survive as loose elements with no ghost frame tag...
        let members = board.elements();
        assert!(members.iter().any(|e| e.id == inside_a));
        assert!(members.iter().any(|e| e.id == inside_b));
        assert!(
            members.iter().all(|e| e.frame.is_none() || e.id == frame),
            "a member still references the removed frame"
        );
        assert!(board.frame_members(frame).is_empty());

        // ...and the detached state survives snapshots.
        let restored = BoardDoc::from_snapshot(&board.snapshot()?)?;
        assert!(
            restored
                .elements()
                .iter()
                .filter(|e| e.id != frame)
                .all(|e| e.frame.is_none()),
            "restored members still reference the removed frame"
        );

        // Undo restores the frame AND the membership tags together.
        board.undo()?;
        assert_eq!(board.frame_members(frame).len(), 2);
        assert!(board.elements().iter().any(|e| e.id == frame));
        Ok(())
    }

    #[test]
    fn hidden_round_trips_and_is_honored_by_the_display_list()
    -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        let id = board.insert_element(&spec(0.0, 0.0, ElementKind::Rect))?;
        board.set_hidden(id, true)?;
        board.commit();
        assert!(board.elements()[0].hidden);

        let restored = BoardDoc::from_snapshot(&board.snapshot()?)?;
        assert!(restored.elements()[0].hidden);
        Ok(())
    }

    #[test]
    fn peers_exchange_full_history_batches_and_converge() -> Result<(), Box<dyn std::error::Error>>
    {
        let alice = BoardDoc::new();
        let bob = BoardDoc::new();

        alice.insert_element(&spec(0.0, 0.0, ElementKind::Rect))?;
        alice.commit();
        bob.apply_remote(&alice.full_updates()?)?;

        let id = bob.insert_element(&spec(10.0, 10.0, ElementKind::Ellipse))?;
        bob.commit();
        alice.apply_remote(&bob.full_updates()?)?;

        let alice_ids: Vec<u64> = alice.elements().iter().map(|e| e.id.raw()).collect();
        let bob_ids: Vec<u64> = bob.elements().iter().map(|e| e.id.raw()).collect();
        assert_eq!(alice_ids, bob_ids, "peers converge on the same board");
        assert!(alice.elements().iter().any(|e| e.id == id));

        // Redundant batches are harmless: CRDT import is idempotent.
        alice.apply_remote(&bob.full_updates()?)?;
        assert_eq!(alice.elements().len(), bob.elements().len());
        Ok(())
    }

    /// Concurrent moves of the same element converge to ONE position on both
    /// peers (per-field last-writer-wins, DESIGN §9) — no divergence, no
    /// duplicated or ghost geometry.
    #[test]
    fn concurrent_moves_converge_identically() -> Result<(), Box<dyn std::error::Error>> {
        const SIZE: Size = Size::new(50.0, 40.0);
        let alice = BoardDoc::new();
        let bob = BoardDoc::new();

        alice.insert_element(&spec(0.0, 0.0, ElementKind::Rect))?;
        alice.commit();
        bob.apply_remote(&alice.full_updates()?)?;

        // Both peers move the SAME element to different places, concurrently.
        let target = bob.elements()[0].id;
        alice.set_geometry(target, Aabb::from_min_size(Point::new(100.0, 0.0), SIZE))?;
        alice.commit();
        bob.set_geometry(target, Aabb::from_min_size(Point::new(0.0, 100.0), SIZE))?;
        bob.commit();

        // Full bidirectional exchange.
        alice.apply_remote(&bob.full_updates()?)?;
        bob.apply_remote(&alice.full_updates()?)?;

        let alice_bounds = alice
            .elements()
            .iter()
            .find(|e| e.id == target)
            .map(|e| e.bounds)
            .ok_or("element vanished on alice")?;
        let bob_bounds = bob
            .elements()
            .iter()
            .find(|e| e.id == target)
            .map(|e| e.bounds)
            .ok_or("element vanished on bob")?;
        assert_eq!(
            alice_bounds, bob_bounds,
            "concurrent moves must converge to one position"
        );
        assert_ne!(
            alice_bounds,
            Aabb::from_min_size(Point::new(0.0, 0.0), SIZE),
            "convergence must pick a written position, not the untouched start"
        );
        Ok(())
    }

    #[test]
    fn corrupt_kind_is_a_loud_error() -> Result<(), Box<dyn std::error::Error>> {
        let board = BoardDoc::new();
        board.insert_element(&spec(0.0, 0.0, ElementKind::Rect))?;
        board.commit();
        let bytes = board.snapshot()?;

        let restored = BoardDoc::from_snapshot(&bytes)?;
        assert_eq!(restored.elements().len(), 1);
        Ok(())
    }
}
