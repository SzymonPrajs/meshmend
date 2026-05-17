#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Rendered,
    Wireframe,
    SurfaceWire,
    XrayWire,
    Transparent,
    Normals,
    Studio,
    Headlight,
    Fit,
    Reset,
}

pub fn draw_icon(painter: &egui::Painter, rect: egui::Rect, icon: Icon, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.9, color);
    let soft = egui::Stroke::new(1.5, color.gamma_multiply(0.62));
    let p = |x: f32, y: f32| -> egui::Pos2 {
        egui::pos2(
            rect.left() + rect.width() * (x / 24.0),
            rect.top() + rect.height() * (y / 24.0),
        )
    };
    let r = |x1: f32, y1: f32, x2: f32, y2: f32| -> egui::Rect {
        egui::Rect::from_min_max(p(x1, y1), p(x2, y2))
    };
    let line = |from: egui::Pos2, to: egui::Pos2, stroke: egui::Stroke| {
        painter.line_segment([from, to], stroke);
    };

    match icon {
        Icon::Rendered => {
            cube(p, painter, stroke, soft);
            line(p(12.0, 3.0), p(12.0, 12.0), soft);
            line(p(5.0, 8.0), p(12.0, 12.0), soft);
            line(p(19.0, 8.0), p(12.0, 12.0), soft);
        }
        Icon::Wireframe => {
            cube(p, painter, stroke, stroke);
        }
        Icon::SurfaceWire => {
            cube(p, painter, stroke, soft);
            line(p(7.0, 10.0), p(16.0, 15.0), soft);
            line(p(9.0, 14.0), p(18.0, 9.0), soft);
            line(p(5.0, 12.0), p(12.0, 16.0), soft);
            line(p(12.0, 16.0), p(19.0, 12.0), soft);
        }
        Icon::XrayWire => {
            cube(p, painter, soft, soft);
            line(p(6.0, 17.0), p(18.0, 7.0), stroke);
            line(p(8.0, 7.0), p(16.0, 17.0), stroke);
            painter.circle_stroke(p(12.0, 12.0), rect.width() * 0.15, stroke);
        }
        Icon::Transparent => {
            painter.rect_stroke(r(5.0, 8.0, 15.0, 18.0), egui::Rounding::same(2.0), soft);
            painter.rect_stroke(r(9.0, 5.0, 19.0, 15.0), egui::Rounding::same(2.0), stroke);
            line(p(9.0, 15.0), p(15.0, 15.0), soft);
            line(p(15.0, 8.0), p(15.0, 18.0), soft);
        }
        Icon::Normals => {
            line(p(5.0, 17.0), p(19.0, 17.0), stroke);
            for x in [7.0, 12.0, 17.0] {
                line(p(x, 17.0), p(x, 7.0), stroke);
                line(p(x, 7.0), p(x - 2.0, 10.0), stroke);
                line(p(x, 7.0), p(x + 2.0, 10.0), stroke);
            }
        }
        Icon::Studio => {
            cube(p, painter, soft, soft);
            sparkle(painter, p(7.0, 6.0), rect.width() * 0.12, stroke);
            sparkle(painter, p(18.0, 8.0), rect.width() * 0.09, stroke);
            painter.circle_stroke(p(12.0, 4.0), rect.width() * 0.08, soft);
        }
        Icon::Headlight => {
            painter.rect_stroke(r(7.0, 7.0, 15.0, 15.0), egui::Rounding::same(2.0), stroke);
            line(p(15.0, 10.0), p(20.0, 7.0), stroke);
            line(p(15.0, 12.0), p(20.0, 15.0), stroke);
            line(p(8.0, 17.0), p(16.0, 17.0), soft);
            line(p(12.0, 15.0), p(12.0, 17.0), soft);
        }
        Icon::Fit => {
            line(p(5.0, 10.0), p(5.0, 5.0), stroke);
            line(p(5.0, 5.0), p(10.0, 5.0), stroke);
            line(p(14.0, 5.0), p(19.0, 5.0), stroke);
            line(p(19.0, 5.0), p(19.0, 10.0), stroke);
            line(p(19.0, 14.0), p(19.0, 19.0), stroke);
            line(p(19.0, 19.0), p(14.0, 19.0), stroke);
            line(p(10.0, 19.0), p(5.0, 19.0), stroke);
            line(p(5.0, 19.0), p(5.0, 14.0), stroke);
        }
        Icon::Reset => {
            painter.circle_stroke(p(12.0, 12.0), rect.width() * 0.31, stroke);
            line(p(7.0, 8.0), p(7.0, 4.0), stroke);
            line(p(7.0, 4.0), p(11.0, 4.0), stroke);
            line(p(7.0, 4.0), p(9.0, 7.0), stroke);
        }
    }
}

fn cube(
    p: impl Fn(f32, f32) -> egui::Pos2,
    painter: &egui::Painter,
    stroke: egui::Stroke,
    inner: egui::Stroke,
) {
    let a = p(12.0, 3.5);
    let b = p(19.0, 7.5);
    let c = p(19.0, 16.0);
    let d = p(12.0, 20.0);
    let e = p(5.0, 16.0);
    let f = p(5.0, 7.5);
    for [from, to] in [[a, b], [b, c], [c, d], [d, e], [e, f], [f, a]] {
        painter.line_segment([from, to], stroke);
    }
    painter.line_segment([f, p(12.0, 12.0)], inner);
    painter.line_segment([b, p(12.0, 12.0)], inner);
    painter.line_segment([d, p(12.0, 12.0)], inner);
}

fn sparkle(painter: &egui::Painter, center: egui::Pos2, radius: f32, stroke: egui::Stroke) {
    painter.line_segment(
        [
            egui::pos2(center.x, center.y - radius),
            egui::pos2(center.x, center.y + radius),
        ],
        stroke,
    );
    painter.line_segment(
        [
            egui::pos2(center.x - radius, center.y),
            egui::pos2(center.x + radius, center.y),
        ],
        stroke,
    );
}
