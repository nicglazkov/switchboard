# Writing and drawing the docs

The site follows one model closely: short definitions, a fixed order of sections, plain second-person
sentences, one kind of callout, and drawn figures in a single quiet style. The reference page is
`docs/concepts/what-is-switchboard.md`; open it beside whatever you write. This file is not part of the site.

Preview: `SB_MKDOCS=<path to mkdocs> design-lab/docs-preview.sh` builds the site and puts it on the lab at
`/docs/`. Shoot a page with `design-lab/shots.sh <out> "/docs/<path>/|name.png|1280|2400|light"` (and `|dark`),
then look at the picture. Every page with a figure is looked at in both themes before it is done.

## Accuracy comes first

Every statement is checked against the code or the running product, not against an older page. Name the file
you checked when you are unsure and leave nothing guessed. If the product does not do something, the page does
not say it does. Menu names, button labels, column names, defaults and limits are copied from the source
(`native/crates/sb-desktop`, `server/`, `agent/sb.py`, `ios/`, `android-native/`).

## Kinds of page

**Feature page** (most pages: one thing the product does). In this order, leaving out a section only when it
would be empty:

1. An availability callout if it applies ("Terminals need a Mac or Linux home computer.").
2. One to three sentences that define the thing: "X lets you ...".
3. `## Why it matters` (optional, short).
4. `## Benefits`, bullets shaped "**Bold lead**: one sentence."
5. `## Use cases`, same shape.
6. `## How it works`, with a figure when there is something to draw.
7. `## Get started` or `## Use X`: an overview list of the steps, `### Prerequisites` ("Before you can ..., you
   must:"), then one `###` heading per step, written as an instruction ("Open the chat's terminal").
8. `## Change ...`, `## Turn off ...` where they exist.
9. `## Limitations`.
10. `## Troubleshooting`, one `###` per symptom, when the feature has known ones.

**Concept page** (an idea: warm and cold, chat states, the home computer). A lead paragraph that defines the
term, an `##` per part, at most one analogy on the page, at most one callout for what happens when it fails. No
steps. Thirteen to twenty short paragraphs.

**Reference page** (commands, files, ports, shortcuts, glossary). Tables for matrices; an `###` per command
with a one-line description, a synopsis block, "Common flags:" bullets with defaults stated, and a link to the
full topic. Glossary: one `##` per term, one to three sentences, "A chat is ...".

**Troubleshooting.** The hub is a list of links grouped by area. Each leaf is named for the symptom ("Can't
see a computer on the board") and gives the cause and the fix in steps.

## Chrome every page has

```markdown
---
description: One sentence for search engines and link previews.
---

# Title in sentence case

<p class="sb-meta">Last validated: Nov 22, 2026</p>

The definition sentence.
```

"Last validated" is the date someone last checked the page against the product, not the date it was edited.

## Voice

- "You" for what the reader does; "Switchboard" as the subject for what the product does. Present tense, active
  voice, contractions ("doesn't", "can't").
- Sentences of 15 to 30 words, paragraphs of two to four sentences, then a list. Any run of three or more
  things is a bulleted list.
- Define a term the first time a page uses it, every page: "one of your computers, known as the home
  computer". Expand an abbreviation in brackets the first time.
- Say why right after a restriction: "This is because ...".
- Cross-reference only as "For more information, refer to [Page title](path.md)." or "Refer to [Page
  title](path.md)." The link text is the page's title. Never "see", never "click here".
- Sentence case headings. Instruction headings for tasks. Interface labels in **bold**, exactly as the app
  spells them. Commands, flags, paths and values in `code`.
- One command per code block, no `$`. Example values that cannot be real (`192.0.2.10`, `example-mac`). After
  it, say what to replace: "Replace `example-mac` with your computer's name."
- No em dashes, no exclamation marks, no emoji. Say "computer" (not machine or node), "home computer" (not
  server, except on the Develop pages and where a command or file is literally named that), "chat" (not
  session). The Develop pages may use the code's own names.
- Plain enough for someone who has never opened a terminal on the Start and Use pages.

## Components

Callout, one kind, no title. The sentence carries the meaning:

```markdown
!!! note ""
    Terminals need a Mac or Linux home computer.
```

Use `!!! tip ""` (green) only to restate good news ("You can skip this step if ..."). Nothing else.

Per-system instructions go in tabs under the step, never as repeated sections:

```markdown
=== "macOS"

    Steps for the Mac.

=== "Windows"

    Steps for Windows.
```

Index cards (section landing pages, the home page):

```markdown
<div class="grid cards" markdown>

-   [The board](board.md)

    Every chat on every computer, sorted by what it needs.

</div>
```

Status words in running text: `<span class="sb-pill need">Needs you</span>`, and `run`, `warm`, `cold`.

Screenshots come in pairs and each theme shows its own:
`![Alt sentence](screenshots/board-light.png){ .shot }` followed by the same with `-dark`.

## Figures

A figure earns its place when it shows a relationship that a sentence can't: who talks to whom, what happens in
what order, before and after. Concept and feature pages get one or two. Reference pages get none.

Each figure is an SVG file in `docs/diagrams/`, placed inline so it takes the page's colours:

```markdown
<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/with-switchboard.svg"
<figcaption>Figure 1(b). One sentence that says what the reader should notice.</figcaption>
</figure>
```

Rules for the file:

- `<svg viewBox="0 0 880 H" role="img" aria-labelledby="ID" xmlns="http://www.w3.org/2000/svg">` then a
  `<title id="ID">` that is a full sentence describing the picture. 880 wide always; height as needed (300 to
  480). **No blank lines anywhere in the file**, no `<style>`, no `fill=` or `stroke=` attributes, no ids
  other than the title's (ids must be unique across the site: use `fig-<file name>-t`).
- Colour comes only from classes, so light and dark both work. One meaning per colour, in every figure:
  teal (`sel`) is Switchboard or the thing in focus; gold (`req`) is a request going out; green (`ret`) is the
  answer coming back; orange (`bad`) is the slow or broken way. Most of a figure is neutral.
- A 40px margin. A title top left: `<text class="d-title" x="40" y="52">`. Align everything to a 20px grid.
  Text never touches or crosses a line or an edge: leave 12px.
- Labels sit beside or under what they name. Nothing is explained only by colour.
- Before and after are two files with the same layout and positions, captioned "Figure 1(a)" and "Figure 1(b)".
- An ordered flow has numbered circles on its arrows (`<circle class="d-num-req" r="13"/>` and
  `<text class="d-num" dy="4.5">1</text>`), gold on the way out and green on the way back, and the text below
  the figure walks through the numbers.
- Arrowheads are open chevrons drawn as part of the path, for example a right-pointing head at (x, y):
  `<path class="d-line" d="M{x-9} {y-8}L{x} {y}L{x-9} {y+8}"/>`. No markers.
- What the figure does not draw (for example that the link is encrypted) is said in the text.

Classes (defined in `docs/stylesheets/extra.css`):

| Class | Use |
|---|---|
| `d-panel` | a white region: a computer, a network, a group. `rx="14"` |
| `d-panel-line` | the same with a hairline |
| `d-dash` | a dashed outline for something virtual or optional |
| `d-chip`, `d-chip-2` | a pill for a named thing (`rx` = half its height) |
| `d-chip-sel`, `d-chip-bad` | a pill or region in focus, or the broken one |
| `d-node`, `d-node-sel`, `d-node-bad`, `d-soft` | a solid square or circle that stands for a device or person |
| `d-ico` | an icon's strokes; add `on-node` on a `d-node`, `on-sel` on a `d-node-sel`, `sel` for teal strokes |
| `d-line`, `d-line-sel`, `d-req`, `d-ret`, `d-bad` | connections; add `d-dotted` for a fallback path |
| `d-num-req`, `d-num-ret`, `d-num-sel`, `d-num-bad` | the numbered circles; `text.d-num` inside |
| `text` | 15px labels. `d-title` the figure title, `d-sub` a quiet second line, `d-small` 13px, `d-mono` for names and commands, `d-code` for short tracked capitals |
| `t-sel`, `t-req`, `t-ret`, `t-bad`, `on-node`, `on-sel` | text colours |
| `mid`, `end` | text anchors |

Icons are drawn on a 24px grid, one stroke weight, and placed with
`<g class="d-ico" transform="translate(X Y)">...</g>`. Use these; add a new one only in the same manner:

```
monitor   <rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>
laptop    <rect x="4" y="5" width="16" height="11" rx="2"/><path d="M2 19h20"/>
tower     <rect x="4" y="4" width="16" height="7" rx="2"/><rect x="4" y="13" width="16" height="7" rx="2"/><path d="M8 7.5h.01M8 16.5h.01"/>
phone     <rect x="7" y="3" width="10" height="18" rx="2.5"/><path d="M11 18h2"/>
person    <circle cx="12" cy="8" r="3.5"/><path d="M5 20c.8-3.6 3.6-5.5 7-5.5s6.2 1.9 7 5.5"/>
board     <rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16M15 4v16"/>
chat      <path d="M5 5h14a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-7l-4 3v-3H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2z"/>
terminal  <rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 9l3 3-3 3M13 15h4"/>
folder    <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>
file      <path d="M7 3h7l4 4v12a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"/><path d="M14 3v4h4"/>
clock     <circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>
snow      <path d="M12 3v18M4.2 7.5l15.6 9M19.8 7.5l-15.6 9"/>
flame     <path d="M12 3c1 3 4.5 5 4.5 9.5a4.5 4.5 0 0 1-9 0c0-1.8.8-3 1.8-4 .3 1.5 1 2.2 1.7 2.5C10.500 8.500 11 5.500 12 3z"/>
key       <circle cx="8" cy="14" r="4"/><path d="M11 11l8-8M16 6l2.5 2.5M14 8l2 2"/>
lock      <rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>
globe     <circle cx="12" cy="12" r="8.5"/><path d="M3.500 12h17M12 3.500c2.500 2.500 3.500 5.300 3.500 8.500s-1 6-3.500 8.500c-2.500-2.500-3.500-5.300-3.500-8.500s1-6 3.500-8.500z"/>
bell      <path d="M6 16V11a6 6 0 0 1 12 0v5l1.500 2h-15z"/><path d="M10 20.500a2 2 0 0 0 4 0"/>
gauge     <path d="M4 16a8 8 0 1 1 16 0"/><path d="M12 16l4-5"/>
check     <path d="M5 12.500l4.500 4.500L19 7.500"/>
question  <circle cx="12" cy="12" r="8.500"/><path d="M9.500 9.500a2.500 2.500 0 1 1 3.500 2.300c-.7.400-1 .9-1 1.700M12 16.500v.2"/>
summary   <rect x="5" y="3" width="14" height="18" rx="2"/><path d="M9 8h6M9 12h6M9 16h3"/>
```
