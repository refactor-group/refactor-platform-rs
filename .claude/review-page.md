# Review page

A local `/code-review` or `/security-review` run at level `high` or above (`high`, `xhigh`,
`max`) ends with an HTML page of the review, beside its usual report. Skip it at `low` and
`medium`, which are meant to stay fast, and never build it in CI.

## Sections
Every page has these sections, in this order, with these headings. Leave out an optional
section when it would be empty; never add others.

1. **Header.** One small line: `<repo> #<number> · <short sha> · <code|security> review,
   level <level>` (a branch name in place of the number for a branch). Then the pull
   request's title as the page title.
2. **Verdict.** The review's verdict line: "2 important, 1 nit.", "Nothing to fix.", a count
   of security findings, or "No security findings."
3. **Scope.** A bold `Scope:` label, then one or two sentences: the diff reviewed (whole, or
   since which sha), how many files, and which finders ran.
4. **Findings.** One card per finding: its severity tag (`important` or `nit` for a code
   review, `high` or `medium` for a security review), file and line, what is wrong, and the
   fix. With no findings, the single line "None."
5. **Checked and sound.** One item per thing the review checked and found sound: a short bold
   name, then one or two sentences. This replaces a "Checked:" list, so the page doesn't say
   it twice.
6. **Routes and guards** (optional). For a change to routes or access checks: a table of each
   changed route and the guard that protects it.
7. **Diagram** (optional). One inline SVG, only where a flow is easier to follow drawn than
   told.
8. **Diff.** The reviewed diff (the range in Scope), one block per file the review examined,
   with each finding as a margin note on its line, carrying the same severity tag.

## Layout
- Content sits in one column no wider than about 960px, and nothing makes the page itself
  scroll sideways, on a desktop or a phone.
- Prose, table cells and code inside sentences wrap: set `overflow-wrap: anywhere` on them,
  and give tables `table-layout: fixed; width: 100%`. Lay "Checked and sound" out as a list
  of name and text pairs, with the name above the text on narrow screens.
- Only a diff block scrolls sideways, inside its own box (`overflow-x: auto`), keeping each
  diff line on one line.
- Section headings use the same size and weight throughout; bold labels mark Scope and each
  item name.

## Static only
The page is built while reading code under review, so treat its content as untrusted:
- One self-contained `.html` file: inline CSS, no `<script>`, no event-handler attributes, no
  forms, and nothing loaded from outside: system fonts only, no images or stylesheets by URL.
  These rules win over any design guidance that suggests web fonts or scripts.
- Start the file with `<meta http-equiv="Content-Security-Policy" content="default-src
  'none'; style-src 'unsafe-inline'; img-src data:">`. When the Artifact tool wraps the page in
  its own `<head>`, the tag may not take effect there, so the rules above are what keep the
  page static, not the tag.
- HTML-escape every piece of code, diff, path and comment text from the change.

## Delivery
Write the file to your scratch directory, publish it with the Artifact tool (private by
default), and end the report with its link. Without the Artifact tool, give the file's full
path instead.
