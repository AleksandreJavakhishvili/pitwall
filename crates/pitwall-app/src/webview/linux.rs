//! The linux backend of [`super::WebView`] (to be built).

use gpui::Window;

use super::{Options, WebView};

pub fn create(_window: &mut Window, _opts: Options) -> Result<Box<dyn WebView>, String> {
    Err("The in-app preview isn't built for this platform yet".into())
}
