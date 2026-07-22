use bevy_egui::egui;

pub fn sparkline(
    ui: &mut egui::Ui,
    data: &[f32],
    size: egui::Vec2,
    color: egui::Color32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());

    if !ui.is_rect_visible(rect) || data.is_empty() {
        return response;
    }

    let painter = ui.painter_at(rect);

    let max_val = data.iter().cloned().fold(1.0_f32, f32::max);
    let min_val = data.iter().cloned().fold(max_val, f32::min);
    let range = (max_val - min_val).max(1.0);

    let n = data.len();
    let points: Vec<egui::Pos2> = data
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = rect.left() + (i as f32 / (n.max(2) - 1) as f32) * rect.width();
            let y = rect.bottom() - ((v - min_val) / range) * rect.height();
            egui::pos2(x, y)
        })
        .collect();

    // Fill area below the line
    let fill_color = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 30);
    let mut fill_points = points.clone();
    fill_points.push(egui::pos2(rect.right(), rect.bottom()));
    fill_points.push(egui::pos2(rect.left(), rect.bottom()));

    let fill_mesh = {
        let mut mesh = egui::Mesh::default();
        if fill_points.len() >= 3 {
            let base_idx = 0;
            for i in 1..fill_points.len() - 1 {
                mesh.colored_vertex(fill_points[base_idx], fill_color);
                mesh.colored_vertex(fill_points[i], fill_color);
                mesh.colored_vertex(fill_points[i + 1], fill_color);
                let idx = mesh.vertices.len() as u32;
                mesh.indices.push(idx - 3);
                mesh.indices.push(idx - 2);
                mesh.indices.push(idx - 1);
            }
        }
        mesh
    };
    painter.add(egui::Shape::mesh(fill_mesh));

    // Draw the line
    if points.len() >= 2 {
        painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, color)));
    }

    // Current value label (right side)
    if let Some(&last) = data.last() {
        let label = format!("{:.0}", last);
        painter.text(
            egui::pos2(rect.right() - 2.0, rect.top() + 2.0),
            egui::Align2::RIGHT_TOP,
            label,
            egui::FontId::monospace(9.0),
            color,
        );
    }

    response
}
