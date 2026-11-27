# Native building blocks

**Preview.** Some parts of Pitwall are too big for a tile to rebuild from boxes and text: its code view and its diff view. A tile embeds them as elements with content it supplies, and they look and behave exactly as they do in Pitwall's files and review tiles: highlighting, line numbers, selection and copy, search, folding of unchanged runs. The tile gives the text; Pitwall does the drawing.

Everything below comes from `use pitwall_extension::prelude::*;`. Both views are read-only.

## Code view

```rust
code_view(source)
    .id("code")
    .language("rust")
    .flex_1()
    .mark(4..7, Mark::Added)
    .reveal(12)
    .on_line_click(cx.listener(|t: &mut MyTile, l: &LineClick, cx| {
        t.line = Some(l.line);
        cx.notify();
    }))
```

| | |
|---|---|
| `code_view(text)` | The view of `text` (at most 1 MiB). Give it a size: `.flex_1()`, `.size_full()` or a height. It scrolls inside |
| `.language(name)` | For highlighting: a name (`rust`, `typescript`, `python`, `go`, `markdown`, …) or a file name (`main.rs`). Unknown names show plain text |
| `.mark(lines, mark)` | A bar in the margin by lines (0-based, end exclusive): `Mark::Added` (green), `Mark::Changed` (blue), or `Mark::Deleted` (a red mark where lines were removed) |
| `.reveal(line)` | Scroll that line (0-based) into the middle. Only when it differs from the last render's, so the user's own scrolling isn't undone |
| `.on_line_click(handler)` | A click on a line number. `LineClick` has the `line` (0-based) and the `side` |
| `.on_select(handler)` | The selection changed. `CodeSelection` has where it starts and ends (line and column, 0-based) and the selected text |

## Diff view

```rust
diff_view(old, new).id("diff").language("rust").flex_1()
diff_view_unified(patch).id("diff").inline()
```

| | |
|---|---|
| `diff_view(old, new)` | The diff of two texts (each at most 1 MiB): changed lines and the characters that changed in them, with unchanged runs folded |
| `diff_view_unified(text)` | A unified diff of one file, as `git diff` prints it: `@@` hunks of `+`, `-` and context lines. Lines keep their numbers from the hunk headers, and what the diff doesn't show is folded |
| `.language(name)` | As for the code view |
| `.inline()` / `.side_by_side()` | One column with removed lines above added ones, or old on the left and new on the right (the default) |
| `.reveal(line)` | Scroll a line of the new version into the middle |
| `.on_line_click(handler)`, `.on_select(handler)` | As for the code view. `side` says which version: `Side::Old` or `Side::New` |

## How they are sent

The text is part of the frame, so it counts toward the frame's 2 MiB, and the SDK sends it with each render. Pitwall reloads a view only when its text, language, layout or marks change, so a render that changes something else in the tile keeps the view's scroll position and selection. Keep a view's key the same across renders.

The selection is told at most every 100 ms while the user drags it, and its text is cut at 64 KiB.

## Testing

The harness shows the views in the text snapshot (`code-view language=rust lines=8 marks=3`, `diff-view texts side-by-side`) and acts on them:

| Call | Does |
|---|---|
| `t.code_view(key)`, `t.diff_view(key)` | What the tile sent: text, language, marks, layout |
| `t.click_line(key, Side::New, 4)` | Click a line number |
| `t.select_code(key, Side::New, (1, 3), (1, 8))` | Select from (line, column) to (line, column) |

## Coming later

A terminal, the file tree, a web view and a Markdown view come in later phases.
