use anyhow::Result;
use ratatui::{buffer::Buffer, style::Color};
use std::path::Path;

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn color(value: Color, background: bool) -> String {
    match value {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Black => "#000000".into(),
        Color::White => "#ffffff".into(),
        _ => if background { "#11161c" } else { "#dce5e7" }.into(),
    }
}
pub fn svg(buffer: &Buffer, path: &Path) -> Result<()> {
    let width = buffer.area.width * 10;
    let height = buffer.area.height * 20;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\"><title>Tree Rail TUI</title><rect width=\"{width}\" height=\"{height}\" fill=\"#11161c\"/><g font-family=\"Cascadia Mono,Consolas,monospace\" font-size=\"15\">"
    );
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            let bg = color(cell.bg, true);
            if bg != "#11161c" {
                svg.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"10\" height=\"20\" fill=\"{bg}\"/>",
                    x * 10,
                    y * 20
                ));
            }
            if cell.symbol() != " " {
                svg.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" fill=\"{}\">{}</text>",
                    x * 10,
                    y * 20 + 15,
                    color(cell.fg, false),
                    xml(cell.symbol())
                ));
            }
        }
    }
    svg.push_str("</g></svg>");
    std::fs::write(path, svg)?;
    Ok(())
}
