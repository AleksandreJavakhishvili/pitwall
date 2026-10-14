//! Agent logos: the mark of each agent kind's tool, drawn before its name
//! in the sidebar, tile headers and New agent. One-colour marks served by
//! the kit's [`Assets`](super::Assets) as `logos/<id>.svg` and tinted like
//! the stroke icons: in the tool's brand colour, else the theme's ink.
//!
//! Which one is the kind's definition's (`icon` in docs/reference/agents.md,
//! never decided per kind here): a bundled logo's id, `terminal`, or an SVG
//! file (`logos/file/<path>`). A kind without one shows its first letter on
//! a badge.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::RwLock;

use gpui::{div, px, svg, AnyElement, FontWeight, Hsla, IntoElement, ParentElement, SharedString, Styled};
use pitwall_core::kind::AgentKind;

use crate::theme::Theme;

#[derive(Clone, Copy)]
enum Tint {
    /// The tool's own colour (both themes).
    Brand(u32),
    /// The theme's secondary ink (black marks).
    Ink,
}

struct Logo {
    id: &'static str,
    tint: Tint,
    /// `fill-rule="evenodd"`.
    evenodd: bool,
    /// One path on a 24×24 view box.
    path: &'static str,
}

/// The bundled logos, by the id definitions name them with.
const LOGOS: &[Logo] = &[
    // Claude (simple-icons, CC0).
    Logo {
        id: "claude",
        tint: Tint::Brand(0xd97757),
        evenodd: false,
        path: "m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z",
    },
    // OpenAI (simple-icons, CC0).
    Logo {
        id: "codex",
        tint: Tint::Ink,
        evenodd: false,
        path: "M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",
    },
    // Google Gemini (simple-icons, CC0).
    Logo {
        id: "gemini",
        tint: Tint::Brand(0x4c8df6),
        evenodd: false,
        path: "M11.04 19.32Q12 21.51 12 24q0-2.49.93-4.68.96-2.19 2.58-3.81t3.81-2.55Q21.51 12 24 12q-2.49 0-4.68-.93a12.3 12.3 0 0 1-3.81-2.58 12.3 12.3 0 0 1-2.58-3.81Q12 2.49 12 0q0 2.49-.96 4.68-.93 2.19-2.55 3.81a12.3 12.3 0 0 1-3.81 2.58Q2.49 12 0 12q2.49 0 4.68.96 2.19.93 3.81 2.55t2.55 3.81",
    },
    // GitHub Copilot (simple-icons, CC0).
    Logo {
        id: "copilot",
        tint: Tint::Ink,
        evenodd: false,
        path: "M23.922 16.997C23.061 18.492 18.063 22.02 12 22.02 5.937 22.02.939 18.492.078 16.997A.641.641 0 0 1 0 16.741v-2.869a.883.883 0 0 1 .053-.22c.372-.935 1.347-2.292 2.605-2.656.167-.429.414-1.055.644-1.517a10.098 10.098 0 0 1-.052-1.086c0-1.331.282-2.499 1.132-3.368.397-.406.89-.717 1.474-.952C7.255 2.937 9.248 1.98 11.978 1.98c2.731 0 4.767.957 6.166 2.093.584.235 1.077.546 1.474.952.85.869 1.132 2.037 1.132 3.368 0 .368-.014.733-.052 1.086.23.462.477 1.088.644 1.517 1.258.364 2.233 1.721 2.605 2.656a.841.841 0 0 1 .053.22v2.869a.641.641 0 0 1-.078.256Zm-11.75-5.992h-.344a4.359 4.359 0 0 1-.355.508c-.77.947-1.918 1.492-3.508 1.492-1.725 0-2.989-.359-3.782-1.259a2.137 2.137 0 0 1-.085-.104L4 11.746v6.585c1.435.779 4.514 2.179 8 2.179 3.486 0 6.565-1.4 8-2.179v-6.585l-.098-.104s-.033.045-.085.104c-.793.9-2.057 1.259-3.782 1.259-1.59 0-2.738-.545-3.508-1.492a4.359 4.359 0 0 1-.355-.508Zm2.328 3.25c.549 0 1 .451 1 1v2c0 .549-.451 1-1 1-.549 0-1-.451-1-1v-2c0-.549.451-1 1-1Zm-5 0c.549 0 1 .451 1 1v2c0 .549-.451 1-1 1-.549 0-1-.451-1-1v-2c0-.549.451-1 1-1Zm3.313-6.185c.136 1.057.403 1.913.878 2.497.442.544 1.134.938 2.344.938 1.573 0 2.292-.337 2.657-.751.384-.435.558-1.15.558-2.361 0-1.14-.243-1.847-.705-2.319-.477-.488-1.319-.862-2.824-1.025-1.487-.161-2.192.138-2.533.529-.269.307-.437.808-.438 1.578v.021c0 .265.021.562.063.893Zm-1.626 0c.042-.331.063-.628.063-.894v-.02c-.001-.77-.169-1.271-.438-1.578-.341-.391-1.046-.69-2.533-.529-1.505.163-2.347.537-2.824 1.025-.462.472-.705 1.179-.705 2.319 0 1.211.175 1.926.558 2.361.365.414 1.084.751 2.657.751 1.21 0 1.902-.394 2.344-.938.475-.584.742-1.44.878-2.497Z",
    },
    // Cursor (LobeHub icons, MIT).
    Logo {
        id: "cursor",
        tint: Tint::Ink,
        evenodd: true,
        path: "M22.106 5.68L12.5.135a.998.998 0 00-.998 0L1.893 5.68a.84.84 0 00-.419.726v11.186c0 .3.16.577.42.727l9.607 5.547a.999.999 0 00.998 0l9.608-5.547a.84.84 0 00.42-.727V6.407a.84.84 0 00-.42-.726zm-.603 1.176L12.228 22.92c-.063.108-.228.064-.228-.061V12.34a.59.59 0 00-.295-.51l-9.11-5.26c-.107-.062-.063-.228.062-.228h18.55c.264 0 .428.286.296.514z",
    },
    // Qwen (LobeHub icons, MIT).
    Logo {
        id: "qwen",
        tint: Tint::Brand(0x6f6bef),
        evenodd: true,
        path: "M12.604 1.34c.393.69.784 1.382 1.174 2.075a.18.18 0 00.157.091h5.552c.174 0 .322.11.446.327l1.454 2.57c.19.337.24.478.024.837-.26.43-.513.864-.76 1.3l-.367.658c-.106.196-.223.28-.04.512l2.652 4.637c.172.301.111.494-.043.77-.437.785-.882 1.564-1.335 2.34-.159.272-.352.375-.68.37-.777-.016-1.552-.01-2.327.016a.099.099 0 00-.081.05 575.097 575.097 0 01-2.705 4.74c-.169.293-.38.363-.725.364-.997.003-2.002.004-3.017.002a.537.537 0 01-.465-.271l-1.335-2.323a.09.09 0 00-.083-.049H4.982c-.285.03-.553-.001-.805-.092l-1.603-2.77a.543.543 0 01-.002-.54l1.207-2.12a.198.198 0 000-.197 550.951 550.951 0 01-1.875-3.272l-.79-1.395c-.16-.31-.173-.496.095-.965.465-.813.927-1.625 1.387-2.436.132-.234.304-.334.584-.335a338.3 338.3 0 012.589-.001.124.124 0 00.107-.063l2.806-4.895a.488.488 0 01.422-.246c.524-.001 1.053 0 1.583-.006L11.704 1c.341-.003.724.032.9.34zm-3.432.403a.06.06 0 00-.052.03L6.254 6.788a.157.157 0 01-.135.078H3.253c-.056 0-.07.025-.041.074l5.81 10.156c.025.042.013.062-.034.063l-2.795.015a.218.218 0 00-.2.116l-1.32 2.31c-.044.078-.021.118.068.118l5.716.008c.046 0 .08.02.104.061l1.403 2.454c.046.081.092.082.139 0l5.006-8.76.783-1.382a.055.055 0 01.096 0l1.424 2.53a.122.122 0 00.107.062l2.763-.02a.04.04 0 00.035-.02.041.041 0 000-.04l-2.9-5.086a.108.108 0 010-.113l.293-.507 1.12-1.977c.024-.041.012-.062-.035-.062H9.2c-.059 0-.073-.026-.043-.077l1.434-2.505a.107.107 0 000-.114L9.225 1.774a.06.06 0 00-.053-.031zm6.29 8.02c.046 0 .058.02.034.06l-.832 1.465-2.613 4.585a.056.056 0 01-.05.029.058.058 0 01-.05-.029L8.498 9.841c-.02-.034-.01-.052.028-.054l.216-.012 6.722-.012z",
    },];

/// Where a user file's logo is served from.
const FILE_PREFIX: &str = "logos/file/";

/// Kind id → the `icon` its definition names (read at startup, like the
/// definitions themselves).
static CHOSEN: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// Remembers the `icon` of each kind that names one.
pub fn learn(kinds: &[AgentKind]) {
    if let Ok(mut c) = CHOSEN.write() {
        *c = Some(icons(kinds));
    }
}

/// Kind id → its definition's `icon`.
fn icons(kinds: &[AgentKind]) -> HashMap<String, String> {
    kinds
        .iter()
        .filter_map(|k| Some((k.id.clone(), expand(k.icon.as_deref()?.trim())?)))
        .collect()
}

/// `~/` becomes the home folder; empty is no icon.
fn expand(icon: &str) -> Option<String> {
    if icon.is_empty() {
        return None;
    }
    if let Some(rest) = icon.strip_prefix("~/") {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
        return Some(std::path::Path::new(&home).join(rest).to_string_lossy().into_owned());
    }
    Some(icon.to_string())
}

fn bundled(id: &str) -> Option<&'static Logo> {
    LOGOS.iter().find(|l| l.id == id)
}

fn logo_svg(l: &Logo) -> String {
    let rule = if l.evenodd { r#" fill-rule="evenodd""# } else { "" };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="#000"><path{rule} d="{}"/></svg>"##,
        l.path
    )
}

/// The asset at `path`, if it is a logo (`logos/<id>.svg`, `logos/file/…`).
pub fn load(path: &str) -> Option<Cow<'static, [u8]>> {
    if let Some(file) = path.strip_prefix(FILE_PREFIX) {
        return std::fs::read(file).ok().map(Cow::Owned);
    }
    let id = path.strip_prefix("logos/")?.strip_suffix(".svg")?;
    bundled(id).map(|l| Cow::Owned(logo_svg(l).into_bytes()))
}

/// The asset paths of the bundled logos.
pub fn list() -> impl Iterator<Item = SharedString> {
    LOGOS.iter().map(|l| format!("logos/{}.svg", l.id).into())
}

/// What a kind is drawn with.
enum Mark {
    Bundled(&'static Logo),
    File(String),
    Terminal,
    Letter(char),
}

fn mark(kind: &str, name: &str) -> Mark {
    let chosen = CHOSEN.read().ok().and_then(|c| c.as_ref()?.get(kind).cloned());
    mark_of(chosen, name)
}

/// The mark for a definition's `icon` (`chosen`), its name `name`.
fn mark_of(chosen: Option<String>, name: &str) -> Mark {
    if let Some(c) = chosen {
        if let Some(l) = bundled(&c) {
            return Mark::Bundled(l);
        }
        if c == "terminal" {
            return Mark::Terminal;
        }
        if c.to_ascii_lowercase().ends_with(".svg") {
            return Mark::File(c);
        }
    }
    Mark::Letter(name.chars().find(|c| c.is_alphanumeric()).unwrap_or('?').to_ascii_uppercase())
}

/// The logo of agent kind `kind` (named `name`), `size` px square.
pub fn agent_logo(kind: &str, name: &str, size: f32, t: &Theme) -> AnyElement {
    let tinted = |path: String, color: Hsla| {
        svg().path(SharedString::from(path)).size(px(size)).flex_none().text_color(color).into_any_element()
    };
    match mark(kind, name) {
        Mark::Bundled(l) => tinted(
            format!("logos/{}.svg", l.id),
            match l.tint {
                Tint::Brand(rgb) => gpui::rgb(rgb).into(),
                Tint::Ink => t.text_2,
            },
        ),
        Mark::File(path) => tinted(format!("{FILE_PREFIX}{path}"), t.text_2),
        Mark::Terminal => super::icon("terminal", size, t.text_2).into_any_element(),
        Mark::Letter(c) => div()
            .flex_none()
            .size(px(size))
            .rounded(px((size * 0.28).round()))
            .bg(t.surface_3)
            .flex()
            .items_center()
            .justify_center()
            .text_size(px((size * 0.66).round()))
            .line_height(px(size))
            .font_weight(FontWeight::BOLD)
            .text_color(t.text_2)
            .child(c.to_string())
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark_in(icons: &HashMap<String, String>, kind: &str, name: &str) -> Mark {
        mark_of(icons.get(kind).cloned(), name)
    }

    #[test]
    fn bundled_logos_load_and_others_fall_back() {
        for l in LOGOS {
            let svg = load(&format!("logos/{}.svg", l.id)).unwrap();
            let s = std::str::from_utf8(&svg).unwrap();
            assert!(s.contains(l.path) && s.starts_with("<svg"), "{}", l.id);
        }
        assert!(load("logos/nope.svg").is_none());
        assert!(matches!(mark_of(None, "drift chat"), Mark::Letter('D')));
        assert!(matches!(mark_of(Some("nope".into()), "  "), Mark::Letter('?')));
    }

    #[test]
    fn a_definition_can_pick_a_logo() {
        let mut k = pitwall_core::kind::custom_kind("claude");
        k.id = "my-claude".into();
        k.icon = Some("claude".into());
        let mut f = k.clone();
        f.id = "drawn".into();
        f.icon = Some("/tmp/x.svg".into());
        let icons = icons(&[k, f]);
        assert!(matches!(mark_in(&icons, "my-claude", "Mine"), Mark::Bundled(l) if l.id == "claude"));
        assert!(matches!(mark_in(&icons, "drawn", "Drawn"), Mark::File(p) if p == "/tmp/x.svg"));
        assert_eq!(expand(""), None);
    }

    #[test]
    fn the_bundled_definitions_name_their_logos() {
        let dir = std::env::temp_dir().join(format!("pitwall-logos-{}", std::process::id()));
        let mut kinds = pitwall_core::kind::KindCatalog::new(dir).kinds();
        kinds.push(pitwall_core::kind::custom_kind(""));
        let icons = icons(&kinds);
        for (kind, name) in [("claude", "Claude Code"), ("codex", "Codex"), ("gemini", "Gemini CLI"), ("copilot", "Copilot"), ("cursor", "Cursor"), ("qwen", "Qwen")] {
            assert!(matches!(mark_in(&icons, kind, name), Mark::Bundled(l) if l.id == kind), "{kind}");
        }
        assert!(matches!(mark_in(&icons, "shell", "Shell"), Mark::Terminal));
        assert!(matches!(mark_in(&icons, "custom", "Custom command"), Mark::Terminal));
        assert!(matches!(mark_in(&icons, "aider", "Aider"), Mark::Letter('A')));
    }
}
