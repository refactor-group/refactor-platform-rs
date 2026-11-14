# Review page

A local `/code-review` or `/security-review` run at level `high` or above (`high`, `xhigh`,
`max`) ends with an HTML page of the review, beside its usual report. Skip it at `low` and
`medium`, which are meant to stay fast, and never build it in CI.

## What it shows
- The reviewed diff (the same range the review covered), rendered per file with each finding
  as a margin note on its line, tagged by severity: `important` or `nit` for a code review,
  `high` or `medium` for a security review. Leave out files the review did not examine.
- One short note per area the review checked and found sound, so a reader sees what was
  covered and not only what failed.
- For a change to routes or access checks, a table of each changed route and the guard that
  protects it.
- A diagram, as inline SVG, only where a flow is easier to follow drawn than told.

## Static only
The page is built while reading code under review, so treat its content as untrusted:
- One self-contained `.html` file: inline CSS, no `<script>`, no event-handler attributes, no
  forms, and nothing loaded from outside (no fonts, images or stylesheets by URL).
- Put `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src
  'unsafe-inline'; img-src data:">` in `<head>`.
- HTML-escape every piece of code, diff, path and comment text from the change.

## Delivery
Write the file to your scratch directory, publish it with the Artifact tool (private by
default), and end the report with its link. Without the Artifact tool, give the file's full
path instead.
