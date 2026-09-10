//! Small vector controls. Their geometry does not depend on the UI font.
use iced::{widget::svg, Theme};

#[derive(Clone, Copy)]
pub enum Icon {
    Close,
    Plus,
    Minus,
    Up,
    Down,
    Right,
    Search,
    Check,
}

pub fn icon<'a>(kind: Icon) -> svg::Svg<'a> {
    let path = match kind {
        Icon::Close => "<path d='M5 5l6 6m0-6l-6 6'/>",
        Icon::Plus => "<path d='M8 2v12M2 8h12'/>",
        Icon::Minus => "<path d='M3 8h10'/>",
        Icon::Up => "<path d='M2 11l6-6 6 6'/>",
        Icon::Down => "<path d='M2 5l6 6 6-6'/>",
        Icon::Right => "<path d='M6 4l4 4-4 4'/>",
        Icon::Search => "<circle cx='6.5' cy='6.5' r='5'/><path d='M10.5 10.5L14 14'/>",
        Icon::Check => "<path d='M3 8l3 3 7-7'/>",
    };
    svg(svg::Handle::from_memory(format!("<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16' viewBox='0 0 16 16'><g fill='none' stroke='black' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'>{path}</g></svg>").into_bytes()))
        .width(16).height(16)
        .style(|theme: &Theme, _| svg::Style { color: Some(theme.palette().text) })
}
