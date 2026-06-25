use bevy_egui::egui;

/// Direction for gradient rendering.
pub enum GradientDirection {
    Vertical,
    Horizontal,
}

/// Paint a two-color gradient rectangle using a vertex-colored mesh.
pub fn paint_gradient_rect(
    painter: &egui::Painter,
    rect: egui::Rect,
    start_color: egui::Color32,
    end_color: egui::Color32,
    direction: GradientDirection,
) {
    let mut mesh = egui::Mesh::default();
    match direction {
        GradientDirection::Vertical => {
            mesh.colored_vertex(rect.left_top(), start_color);
            mesh.colored_vertex(rect.right_top(), start_color);
            mesh.colored_vertex(rect.left_bottom(), end_color);
            mesh.colored_vertex(rect.right_bottom(), end_color);
        }
        GradientDirection::Horizontal => {
            mesh.colored_vertex(rect.left_top(), start_color);
            mesh.colored_vertex(rect.right_top(), end_color);
            mesh.colored_vertex(rect.left_bottom(), start_color);
            mesh.colored_vertex(rect.right_bottom(), end_color);
        }
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Paint a multi-stop gradient rectangle.
pub fn paint_multi_gradient_rect(
    painter: &egui::Painter,
    rect: egui::Rect,
    colors: &[egui::Color32],
    direction: GradientDirection,
) {
    if colors.len() < 2 {
        return;
    }
    let n = colors.len() - 1;
    let mut mesh = egui::Mesh::default();

    for (i, &color) in colors.iter().enumerate() {
        let t = i as f32 / n as f32;
        match direction {
            GradientDirection::Vertical => {
                let y = egui::lerp(rect.top()..=rect.bottom(), t);
                mesh.colored_vertex(egui::pos2(rect.left(), y), color);
                mesh.colored_vertex(egui::pos2(rect.right(), y), color);
            }
            GradientDirection::Horizontal => {
                let x = egui::lerp(rect.left()..=rect.right(), t);
                mesh.colored_vertex(egui::pos2(x, rect.top()), color);
                mesh.colored_vertex(egui::pos2(x, rect.bottom()), color);
            }
        }
        if i < n {
            let idx = (2 * i) as u32;
            mesh.add_triangle(idx, idx + 1, idx + 2);
            mesh.add_triangle(idx + 1, idx + 2, idx + 3);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Generate arc points for gauge rendering.
pub fn arc_points(
    center: egui::Pos2,
    radius: f32,
    start_angle: f32,
    end_angle: f32,
    segments: usize,
) -> Vec<egui::Pos2> {
    (0..=segments)
        .map(|i| {
            let t = i as f32 / segments as f32;
            let angle = start_angle + t * (end_angle - start_angle);
            egui::pos2(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            )
        })
        .collect()
}

/// Paint a stroked arc.
pub fn paint_arc(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    start_angle: f32,
    end_angle: f32,
    stroke: egui::Stroke,
    segments: usize,
) {
    let points = arc_points(center, radius, start_angle, end_angle, segments);
    painter.add(egui::Shape::Path(egui::epaint::PathShape::line(
        points, stroke,
    )));
}

/// Paint a filled arc band (donut segment) using a triangle mesh.
pub fn paint_thick_arc(
    painter: &egui::Painter,
    center: egui::Pos2,
    inner_radius: f32,
    outer_radius: f32,
    start_angle: f32,
    end_angle: f32,
    color: egui::Color32,
    segments: usize,
) {
    let mut mesh = egui::Mesh::default();

    for i in 0..=segments {
        let t = i as f32 / segments as f32;
        let angle = start_angle + t * (end_angle - start_angle);
        let cos_a = angle.cos();
        let sin_a = angle.sin();

        mesh.colored_vertex(
            egui::pos2(
                center.x + inner_radius * cos_a,
                center.y + inner_radius * sin_a,
            ),
            color,
        );
        mesh.colored_vertex(
            egui::pos2(
                center.x + outer_radius * cos_a,
                center.y + outer_radius * sin_a,
            ),
            color,
        );

        if i < segments {
            let base = (i * 2) as u32;
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 2, base + 3);
        }
    }

    painter.add(egui::Shape::mesh(mesh));
}

/// Linear interpolation between two colors in sRGB space.
pub fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let inv = 1.0 - t;
    egui::Color32::from_rgba_unmultiplied(
        (a.r() as f32 * inv + b.r() as f32 * t) as u8,
        (a.g() as f32 * inv + b.g() as f32 * t) as u8,
        (a.b() as f32 * inv + b.b() as f32 * t) as u8,
        (a.a() as f32 * inv + b.a() as f32 * t) as u8,
    )
}
