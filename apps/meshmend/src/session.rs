use std::{
    ops::Range,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use glam::Vec2;
use meshmend_core::{MeshBounds, MeshStats, Triangle, TriangleId};
use meshmend_geometry::{
    split_and_cap_mesh, CapDensity, CutMeshOptions, CutPlane, CutSide, IndexedMesh,
};
use meshmend_render::SelectionOverlay;
use meshmend_stl::{load_binary_stl_with_options, write_binary_stl, LoadOptions};

use crate::commands::{
    vec2_to_array, BoundsSnapshot, CapDensityName, CutPreviewSnapshot, ObjectSnapshot,
};

#[derive(Debug, Clone, Default)]
pub struct MeshSession {
    source_path: Option<PathBuf>,
    display_name: Option<String>,
    stats: Option<MeshStats>,
    triangles: Vec<Triangle>,
    revisions: Vec<MeshRevision>,
    objects: Vec<MeshObject>,
    selected_object: Option<usize>,
    render_index: Vec<usize>,
    dirty: bool,
    pending_cut: Option<PendingCut>,
}

impl MeshSession {
    pub fn load_stl(&mut self, path: &Path) -> Result<MeshLoadReport> {
        let parsed = load_binary_stl_with_options(
            path,
            &LoadOptions {
                parallel: true,
                ..LoadOptions::default()
            },
        )?;
        self.source_path = Some(parsed.source_path.clone());
        self.display_name = Some(parsed.file_name.clone());
        self.stats = Some(parsed.stats.clone());
        self.triangles = parsed
            .chunks
            .iter()
            .flat_map(|chunk| chunk.triangles.iter().copied())
            .collect();
        self.revisions.clear();
        self.objects.clear();
        self.selected_object = None;
        self.render_index = (0..self.triangles.len()).collect();
        self.dirty = false;
        self.pending_cut = None;
        Ok(MeshLoadReport {
            file_name: parsed.file_name,
            stats: parsed.stats,
            chunk_count: parsed.chunks.len(),
            parse_ms: parsed.timings.parse.as_secs_f64() * 1000.0,
        })
    }

    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    pub fn bounds(&self) -> Option<MeshBounds> {
        (!self.triangles.is_empty()).then(|| bounds_for_triangles(&self.triangles))
    }

    pub fn visible_triangles(&mut self) -> Vec<Triangle> {
        self.render_index.clear();
        if self.objects.is_empty() {
            self.render_index.extend(0..self.triangles.len());
            return self.triangles.clone();
        }

        let mut visible = Vec::new();
        for object in self.objects.iter().filter(|object| !object.hidden) {
            for index in object.range.clone() {
                if let Some(triangle) = self.triangles.get(index).copied() {
                    self.render_index.push(index);
                    visible.push(triangle);
                }
            }
        }
        visible
    }

    pub fn visible_triangle_snapshot(&self) -> Vec<Triangle> {
        if self.objects.is_empty() {
            return self.triangles.clone();
        }
        self.objects
            .iter()
            .filter(|object| !object.hidden)
            .flat_map(|object| self.triangles[object.range.clone()].iter().copied())
            .collect()
    }

    pub fn document_triangle_index(&self, render_triangle_index: usize) -> Option<usize> {
        self.render_index.get(render_triangle_index).copied()
    }

    pub fn set_pending_cut(
        &mut self,
        plane: CutPlane,
        start: Vec2,
        end: Vec2,
        segment_count: usize,
        affected_triangles: usize,
    ) {
        self.pending_cut = Some(PendingCut {
            plane,
            start,
            end,
            segment_count,
            affected_triangles,
        });
    }

    pub fn clear_pending_cut(&mut self) {
        self.pending_cut = None;
    }

    pub fn cut_preview_snapshot(&self) -> CutPreviewSnapshot {
        self.pending_cut
            .as_ref()
            .map(|cut| CutPreviewSnapshot {
                active: true,
                segments: cut.segment_count,
                affected_triangles: cut.affected_triangles,
                start: Some(vec2_to_array(cut.start)),
                end: Some(vec2_to_array(cut.end)),
            })
            .unwrap_or_default()
    }

    pub fn apply_pending_cut(
        &mut self,
        cap_density: CapDensityName,
        smooth_cap: bool,
    ) -> Result<CutApplyReport> {
        let Some(cut) = self.pending_cut.clone() else {
            return Err(anyhow!("draw a cut line first"));
        };
        if cut.segment_count == 0 {
            return Err(anyhow!("cut line does not intersect the mesh"));
        }
        if self.triangles.is_empty() {
            return Err(anyhow!("load an STL before cutting"));
        }

        self.push_revision();
        let bounds = bounds_for_triangles(&self.triangles);
        let result = match split_and_cap_mesh(
            &self.triangles,
            cut.plane,
            CutMeshOptions {
                weld_tolerance: bounds.radius().max(1.0) * 1.0e-6,
                target_edge_length: None,
                cap_density: CapDensity::from(cap_density),
                smooth_cap,
            },
        ) {
            Ok(result) => result,
            Err(err) => {
                let _ = self.undo();
                return Err(err.into());
            }
        };

        let components =
            cut_components_from_result(&result.pieces, bounds.radius().max(1.0) * 1.0e-6);
        self.replace_with_components(components);
        self.selected_object = self
            .smallest_capped_object_index()
            .or_else(|| self.smallest_visible_object_index());
        self.dirty = true;
        self.pending_cut = None;

        Ok(CutApplyReport {
            loop_count: result.loops.len(),
            target_edge_length: result.target_edge_length,
            cap_triangles: result
                .pieces
                .iter()
                .map(|piece| piece.cap_triangle_count)
                .sum(),
            warnings: result.warnings,
            object_count: self.objects.len(),
            selected_object: self.selected_object,
        })
    }

    pub fn undo(&mut self) -> bool {
        let Some(revision) = self.revisions.pop() else {
            return false;
        };
        self.triangles = revision.triangles;
        self.objects = revision.objects;
        self.selected_object = revision.selected_object;
        self.render_index = revision.render_index;
        self.dirty = revision.dirty;
        self.pending_cut = None;
        true
    }

    pub fn select_object(&mut self, index: usize) -> Result<()> {
        let Some(object) = self.objects.get(index) else {
            return Err(anyhow!("cut object {index} does not exist"));
        };
        if object.hidden {
            return Err(anyhow!("{} is hidden", object.label));
        }
        self.selected_object = Some(index);
        Ok(())
    }

    pub fn select_object_for_render_triangle(
        &mut self,
        render_triangle_index: usize,
    ) -> Result<usize> {
        let document_triangle_index = self
            .document_triangle_index(render_triangle_index)
            .ok_or_else(|| anyhow!("picked triangle is not in the current render mapping"))?;
        let object_index = self
            .objects
            .iter()
            .position(|object| object.range.contains(&document_triangle_index) && !object.hidden)
            .ok_or_else(|| anyhow!("picked triangle is not part of a selectable cut object"))?;
        self.selected_object = Some(object_index);
        Ok(object_index)
    }

    pub fn selected_object_index(&self) -> Option<usize> {
        self.selected_object
    }

    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    pub fn visible_object_count(&self) -> usize {
        self.objects.iter().filter(|object| !object.hidden).count()
    }

    pub fn hide_selected_object(&mut self) -> Result<String> {
        let selected = self
            .selected_object
            .ok_or_else(|| anyhow!("select a cut object first"))?;
        if self.visible_object_count() <= 1 {
            return Err(anyhow!("cannot hide the last visible object"));
        }
        self.push_revision();
        let label = self.objects[selected].label.clone();
        self.objects[selected].hidden = true;
        self.selected_object = None;
        self.dirty = true;
        Ok(label)
    }

    pub fn show_all_objects(&mut self) -> Result<usize> {
        let hidden_count = self.objects.iter().filter(|object| object.hidden).count();
        if hidden_count == 0 {
            return Ok(0);
        }
        self.push_revision();
        for object in &mut self.objects {
            object.hidden = false;
        }
        self.selected_object = None;
        self.dirty = true;
        Ok(hidden_count)
    }

    pub fn delete_selected_object(&mut self) -> Result<()> {
        let selected = self
            .selected_object
            .ok_or_else(|| anyhow!("select a cut object first"))?;
        self.push_revision();
        let selected_range = self.objects[selected].range.clone();
        self.triangles = self
            .triangles
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, triangle)| (!selected_range.contains(&index)).then_some(triangle))
            .collect();
        self.objects.clear();
        self.selected_object = None;
        self.render_index = (0..self.triangles.len()).collect();
        self.dirty = true;
        Ok(())
    }

    pub fn keep_only_selected_object(&mut self) -> Result<()> {
        let selected = self
            .selected_object
            .ok_or_else(|| anyhow!("select a cut object first"))?;
        self.push_revision();
        let selected_range = self.objects[selected].range.clone();
        self.triangles = self.triangles[selected_range].to_vec();
        self.objects.clear();
        self.selected_object = None;
        self.render_index = (0..self.triangles.len()).collect();
        self.dirty = true;
        Ok(())
    }

    pub fn export_visible(&self, path: &Path) -> Result<()> {
        let triangles = self.visible_triangle_snapshot();
        if triangles.is_empty() {
            return Err(anyhow!("there are no visible triangles to export"));
        }
        write_stl_and_validate(path, &triangles)
    }

    pub fn export_object(&self, index: usize, path: &Path) -> Result<()> {
        let object = self
            .objects
            .get(index)
            .ok_or_else(|| anyhow!("cut object {index} does not exist"))?;
        write_stl_and_validate(path, &self.triangles[object.range.clone()])
    }

    pub fn export_all_objects(&self, directory: &Path) -> Result<usize> {
        if self.objects.is_empty() {
            return Err(anyhow!(
                "cut objects are only available after applying a cut"
            ));
        }
        std::fs::create_dir_all(directory)?;
        let mut exported = 0;
        for (index, object) in self.objects.iter().enumerate() {
            let filename = format!("{:02}-{}.stl", index + 1, object.file_slug());
            let path = directory.join(filename);
            write_stl_and_validate(&path, &self.triangles[object.range.clone()])?;
            exported += 1;
        }
        Ok(exported)
    }

    pub fn object_selection_overlay(&self, index: usize) -> SelectionOverlay {
        let Some(object) = self.objects.get(index) else {
            return SelectionOverlay::default();
        };
        if object.hidden {
            return SelectionOverlay::default();
        }
        SelectionOverlay {
            vertices: Vec::new(),
            edges: Vec::new(),
            faces: self.triangles[object.range.clone()]
                .iter()
                .map(|triangle| triangle.vertices)
                .collect(),
        }
    }

    pub fn cap_overlay(&self) -> SelectionOverlay {
        let mut faces = Vec::new();
        for object in self.objects.iter().filter(|object| !object.hidden) {
            let cap_start = object.range.end.saturating_sub(object.cap_triangle_count);
            for index in cap_start..object.range.end {
                if let Some(triangle) = self.triangles.get(index) {
                    faces.push(triangle.vertices);
                }
            }
        }
        SelectionOverlay {
            vertices: Vec::new(),
            edges: Vec::new(),
            faces,
        }
    }

    pub fn object_snapshots(&self) -> Vec<ObjectSnapshot> {
        self.objects
            .iter()
            .enumerate()
            .map(|(index, object)| ObjectSnapshot {
                index,
                name: object.label.clone(),
                side: cut_side_label(object.side).to_string(),
                visible: !object.hidden,
                selected: self.selected_object == Some(index),
                triangles: object.range.len(),
                cap_triangles: object.cap_triangle_count,
                bounds: object.bounds.into(),
            })
            .collect()
    }

    pub fn bounds_snapshot(&self) -> Option<BoundsSnapshot> {
        self.bounds()
            .filter(|bounds| !bounds.is_empty())
            .map(Into::into)
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    fn push_revision(&mut self) {
        self.revisions.push(MeshRevision {
            triangles: self.triangles.clone(),
            objects: self.objects.clone(),
            selected_object: self.selected_object,
            render_index: self.render_index.clone(),
            dirty: self.dirty,
        });
    }

    fn smallest_visible_object_index(&self) -> Option<usize> {
        self.objects
            .iter()
            .enumerate()
            .filter(|(_, object)| !object.hidden)
            .min_by_key(|(_, object)| object.range.len())
            .map(|(index, _)| index)
    }

    fn smallest_capped_object_index(&self) -> Option<usize> {
        self.objects
            .iter()
            .enumerate()
            .filter(|(_, object)| !object.hidden && object.cap_triangle_count > 0)
            .min_by_key(|(_, object)| object.range.len())
            .map(|(index, _)| index)
    }

    fn replace_with_components(&mut self, components: Vec<CutComponent>) {
        self.triangles.clear();
        self.objects.clear();
        for component in components {
            let start = self.triangles.len();
            self.triangles.extend(component.triangles);
            let end = self.triangles.len();
            let object_index = self.objects.len();
            self.objects.push(MeshObject {
                label: object_label(object_index),
                side: component.side,
                range: start..end,
                cap_triangle_count: component.cap_triangle_count,
                bounds: component.bounds,
                hidden: false,
            });
        }
        self.render_index = (0..self.triangles.len()).collect();
    }
}

#[derive(Debug, Clone)]
pub struct MeshLoadReport {
    pub file_name: String,
    pub stats: MeshStats,
    pub chunk_count: usize,
    pub parse_ms: f64,
}

#[derive(Debug, Clone)]
pub struct CutApplyReport {
    pub loop_count: usize,
    pub target_edge_length: f32,
    pub cap_triangles: usize,
    pub warnings: Vec<String>,
    pub object_count: usize,
    pub selected_object: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct MeshObject {
    pub label: String,
    pub side: CutSide,
    pub range: Range<usize>,
    pub cap_triangle_count: usize,
    pub bounds: MeshBounds,
    pub hidden: bool,
}

impl MeshObject {
    fn file_slug(&self) -> String {
        self.label.replace(' ', "-").to_lowercase()
    }
}

#[derive(Debug, Clone)]
pub struct PendingCut {
    pub plane: CutPlane,
    pub start: Vec2,
    pub end: Vec2,
    pub segment_count: usize,
    pub affected_triangles: usize,
}

#[derive(Debug, Clone)]
struct MeshRevision {
    triangles: Vec<Triangle>,
    objects: Vec<MeshObject>,
    selected_object: Option<usize>,
    render_index: Vec<usize>,
    dirty: bool,
}

#[derive(Debug, Clone)]
struct CutComponent {
    side: CutSide,
    triangles: Vec<Triangle>,
    cap_triangle_count: usize,
    bounds: MeshBounds,
}

fn cut_components_from_result(
    pieces: &[meshmend_geometry::CutPiece],
    weld_tolerance: f32,
) -> Vec<CutComponent> {
    let mut components = Vec::new();
    for piece in pieces {
        components.extend(cut_components_for_side(
            piece.side,
            &piece.triangles,
            piece.cap_triangle_count,
            weld_tolerance,
        ));
    }
    components
}

fn cut_components_for_side(
    side: CutSide,
    triangles: &[Triangle],
    cap_triangle_count: usize,
    weld_tolerance: f32,
) -> Vec<CutComponent> {
    if triangles.is_empty() {
        return Vec::new();
    }
    let mesh = IndexedMesh::from_triangles(
        triangles
            .iter()
            .copied()
            .enumerate()
            .map(|(index, triangle)| {
                (
                    TriangleId {
                        chunk: 0,
                        local_index: index as u32,
                    },
                    triangle,
                )
            }),
        weld_tolerance,
    );
    let component_count = mesh.connectivity.component_count as usize;
    if component_count <= 1 {
        return vec![CutComponent {
            side,
            triangles: triangles.to_vec(),
            cap_triangle_count,
            bounds: bounds_for_triangles(triangles),
        }];
    }

    let mut component_triangles = vec![Vec::new(); component_count];
    let mut component_cap_counts = vec![0_usize; component_count];
    let cap_start = triangles.len().saturating_sub(cap_triangle_count);
    for (face_index, component_id) in mesh.connectivity.component_ids.iter().copied().enumerate() {
        let component_index = component_id as usize;
        if let Some(triangle) = triangles.get(face_index).copied() {
            component_triangles[component_index].push(triangle);
            if face_index >= cap_start {
                component_cap_counts[component_index] += 1;
            }
        }
    }

    component_triangles
        .into_iter()
        .zip(component_cap_counts)
        .filter_map(|(triangles, cap_triangle_count)| {
            (!triangles.is_empty()).then(|| CutComponent {
                side,
                cap_triangle_count,
                bounds: bounds_for_triangles(&triangles),
                triangles,
            })
        })
        .collect()
}

pub fn bounds_for_triangles(triangles: &[Triangle]) -> MeshBounds {
    triangles
        .iter()
        .flat_map(|triangle| triangle.vertices)
        .fold(MeshBounds::EMPTY, |mut bounds, vertex| {
            bounds.include_point(vertex);
            bounds
        })
}

pub fn object_label(index: usize) -> String {
    format!("Object {}", index + 1)
}

fn cut_side_label(side: CutSide) -> &'static str {
    match side {
        CutSide::Positive => "positive",
        CutSide::Negative => "negative",
    }
}

fn write_stl_and_validate(path: &Path, triangles: &[Triangle]) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    write_binary_stl(path, triangles)?;
    load_binary_stl_with_options(
        path,
        &LoadOptions {
            parallel: true,
            ..LoadOptions::default()
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn object_labels_are_not_limited_to_two_pieces() {
        assert_eq!(object_label(0), "Object 1");
        assert_eq!(object_label(1), "Object 2");
        assert_eq!(object_label(29), "Object 30");
    }

    #[test]
    fn components_split_disconnected_triangles() {
        let triangles = vec![test_triangle(0.0), test_triangle(4.0)];

        let components = cut_components_for_side(CutSide::Positive, &triangles, 1, 1.0e-6);

        assert_eq!(components.len(), 2);
        assert_eq!(
            components
                .iter()
                .map(|component| component.triangles.len())
                .sum::<usize>(),
            2
        );
        assert_eq!(
            components
                .iter()
                .map(|component| component.cap_triangle_count)
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn visible_triangles_tracks_render_mapping() {
        let triangles = vec![test_triangle(0.0), test_triangle(2.0), test_triangle(4.0)];
        let mut session = MeshSession {
            triangles: triangles.clone(),
            objects: vec![
                MeshObject {
                    label: object_label(0),
                    side: CutSide::Positive,
                    range: 0..1,
                    cap_triangle_count: 0,
                    bounds: bounds_for_triangles(&triangles[0..1]),
                    hidden: true,
                },
                MeshObject {
                    label: object_label(1),
                    side: CutSide::Negative,
                    range: 1..3,
                    cap_triangle_count: 0,
                    bounds: bounds_for_triangles(&triangles[1..3]),
                    hidden: false,
                },
            ],
            ..MeshSession::default()
        };

        let visible = session.visible_triangles();

        assert_eq!(visible.len(), 2);
        assert_eq!(session.document_triangle_index(0), Some(1));
        assert_eq!(session.document_triangle_index(1), Some(2));
    }

    fn test_triangle(offset: f32) -> Triangle {
        Triangle {
            normal: Vec3::Z,
            vertices: [
                Vec3::new(offset, 0.0, 0.0),
                Vec3::new(offset + 1.0, 0.0, 0.0),
                Vec3::new(offset, 1.0, 0.0),
            ],
        }
    }
}
