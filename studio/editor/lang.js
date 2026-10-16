// editor/lang.js — what the editor knows about the studio's languages, with
// no DOM and no Monaco, so it can be tested in Node: the expr language
// (Monarch grammar, brackets, theme), JSX on top of Monaco's JavaScript
// grammar, the language of a path, the JSON schema that applies to a path,
// and compiler diagnostics → markers. editor.js hands these to Monaco.
//
// Origin: the expr grammar, its language configuration, the theme overrides
// and the idempotent registration follow silvin's
// projects/morty/admin-frontend/src/components/ScriptEditor.tsx (exprlang),
// adapted to the language the compiler accepts (lang/compiler/src/syntax.rs:
// its lexer, OPS and keywords) and to the studio's light palette.

/** Monaco's language id for engine.expr, and the studio's theme. */
export const EXPR = 'expr';
export const THEME = 'studio-dark';

/** Words the parser treats specially (syntax.rs; `if`/`else` per SPEC §2.1). */
export const KEYWORDS = ['let', 'in', 'nil', 'true', 'false', 'and', 'or', 'not', 'if', 'else'];

/**
 * expr's builtins the language accepts (lang/SPEC.md §2.3; the test checks
 * this list against the spec). All of them are built; what each holds is in
 * the expr-engine skill's "Lists" sections.
 */
export const EXPR_BUILTINS = [
  'len', 'sum', 'mean', 'median', 'min', 'max', 'count', 'any', 'all', 'none', 'one',
  'first', 'last', 'find', 'findLast', 'findIndex', 'findLastIndex', 'get',
  'map', 'filter', 'reduce', 'take', 'reverse', 'uniq', 'sortBy', 'sort', 'concat', 'flatten',
  'abs', 'int', 'float', 'round', 'floor', 'ceil',
];

/** The engine profile's host builtins (engine/builtins/builtins.json; the test checks it). */
export const SAILING_BUILTINS = ['distance', 'bearing', 'courses_cross', 'lines_cross', 'course_crosses_line', 'angle_diff'];

/** Brackets, comments and auto-closing pairs for expr (as morty's exprlang, plus block comments). */
export const exprConfiguration = {
  comments: { lineComment: '//', blockComment: ['/*', '*/'] },
  brackets: [['{', '}'], ['[', ']'], ['(', ')']],
  autoClosingPairs: [
    { open: '{', close: '}' }, { open: '[', close: ']' }, { open: '(', close: ')' },
    { open: "'", close: "'", notIn: ['string', 'comment'] }, { open: '"', close: '"', notIn: ['string', 'comment'] },
  ],
  surroundingPairs: [
    { open: '{', close: '}' }, { open: '[', close: ']' }, { open: '(', close: ')' },
    { open: "'", close: "'" }, { open: '"', close: '"' },
  ],
};

/**
 * The expr Monarch grammar. Tokens: keyword, builtin (expr's), builtin.sailing
 * (the host's), predicate (`#`, `#index`, `#acc`), field (after `.`/`?.`, so
 * `s.max` isn't a builtin), identifier, number, string, comment, operator,
 * delimiter, invalid. Mirrors the compiler's lexer: what it lexes only to give
 * a hint (`===`, `=>`, `...`) or never parses (`^`), and any other character,
 * is invalid.
 */
export const exprMonarch = {
  defaultToken: 'invalid',
  keywords: KEYWORDS,
  builtins: EXPR_BUILTINS,
  sailing: SAILING_BUILTINS,
  tokenizer: {
    root: [
      [/[ \t\r\n]+/, ''],
      [/\/\/.*$/, 'comment'],
      [/\/\*/, 'comment', '@comment'],
      // a field after `.` or `?.` (also a predicate's leading `.f`, which is `#.f`)
      [/(\?\.|\.)([A-Za-z_$][\w$]*)/, ['operator', 'field']],
      [/[A-Za-z_$][\w$]*/, { cases: { '@keywords': 'keyword', '@builtins': 'builtin', '@sailing': 'builtin.sailing', '@default': 'identifier' } }],
      [/#[A-Za-z0-9]*/, 'predicate'],
      [/0[xX][0-9a-fA-F_]+/, 'number.hex'],
      [/\d[\d_]*\.\d[\d_]*([eE][+-]?\d+)?/, 'number.float'],
      [/\d[\d_]*[eE][+-]?\d+/, 'number.float'],
      [/\d[\d_]*/, 'number'],
      [/'/, { token: 'string.quote', next: '@string_single' }],
      [/"/, { token: 'string.quote', next: '@string_double' }],
      [/===|=>|\.\.\.|\^/, 'invalid'],
      [/\?\?|\?\.|\*\*|\.\.|==|!=|<=|>=|&&|\|\||[+\-*/%<>!?:=|]/, 'operator'],
      [/[{}()[\]]/, '@brackets'],
      [/[;,.]/, 'delimiter'],
    ],
    comment: [
      [/[^/*]+/, 'comment'],
      [/\*\//, 'comment', '@pop'],
      [/[/*]/, 'comment'],
    ],
    // Strings end at the end of the line, as in the compiler (and morty's
    // grammar), so an unterminated one doesn't paint the rest of the file.
    string_single: [
      [/[^\\'\r\n]+/, 'string'],
      [/\\(?:[ntr\\"']|u[0-9a-fA-F]{4})/, 'string.escape'],
      [/\\./, 'string.invalid'],
      [/'/, { token: 'string.quote', next: '@pop' }],
      [/$/, { token: 'string.invalid', next: '@pop' }],
    ],
    string_double: [
      [/[^\\"\r\n]+/, 'string'],
      [/\\(?:[ntr\\"']|u[0-9a-fA-F]{4})/, 'string.escape'],
      [/\\./, 'string.invalid'],
      [/"/, { token: 'string.quote', next: '@pop' }],
      [/$/, { token: 'string.invalid', next: '@pop' }],
    ],
  },
};

/**
 * JSX on Monaco's JavaScript grammar (which has none: `<div class="x">` is
 * operators and the keyword `class`, and an apostrophe in text opens a
 * string). A tag starts at `<` directly followed by a name and then a space,
 * `/` or `>` (so `a < b` and `i<n;` stay comparisons), or at `<>`. Inside a
 * tag: attribute names and values, `{…}` expressions, `/>` or `>`; then
 * children: text, `{…}`, nested tags, and the closing tag. Takes the language
 * object Monaco's javascript loader gives and returns a new one.
 */
export const JSX_TAG_START = /(<)([A-Za-z][\w$.:-]*)(?=[\s/>]|$)/;
export function withJsx(language) {
  const t = language.tokenizer;
  const open = [JSX_TAG_START, ['delimiter.jsx', { token: 'tag.jsx', next: '@jsxOpen' }]];
  const fragment = [/<>/, { token: 'delimiter.jsx', next: '@jsxChildren' }];
  return {
    ...language,
    tokenizer: {
      ...t,
      common: [fragment, open, ...t.common],
      jsxOpen: [
        [/[ \t\r\n]+/, ''],
        [/\{/, { token: 'delimiter.bracket', next: '@jsxExpr' }],
        [/[A-Za-z_$][\w$.:-]*/, 'attribute.name'],
        [/=/, 'delimiter'],
        [/"[^"]*"/, 'attribute.value'],
        [/'[^']*'/, 'attribute.value'],
        [/\/>/, { token: 'delimiter.jsx', next: '@pop' }],
        [/>/, { token: 'delimiter.jsx', switchTo: '@jsxChildren' }],
        [/\/\/.*$/, 'comment'],
      ],
      jsxChildren: [
        [/[^<{]+/, 'jsx.text'],
        [/\{/, { token: 'delimiter.bracket', next: '@jsxExpr' }],
        [/(<\/)([A-Za-z][\w$.:-]*)(\s*>)/, ['delimiter.jsx', 'tag.jsx', { token: 'delimiter.jsx', next: '@pop' }]],
        [/<\/>/, { token: 'delimiter.jsx', next: '@pop' }],
        fragment,
        [/(<)([A-Za-z][\w$.:-]*)/, ['delimiter.jsx', { token: 'tag.jsx', next: '@jsxOpen' }]],
        [/</, 'delimiter.jsx'],
      ],
      jsxExpr: [
        [/\{/, { token: 'delimiter.bracket', next: '@jsxExpr' }],
        [/\}/, { token: 'delimiter.bracket', next: '@pop' }],
        { include: 'common' },
      ],
    },
  };
}

/**
 * The studio's dark theme. Colours from index.html's tokens: ground #111418,
 * chrome #14181D, hover #222831, selected #262C35 / #2A313B, lines #262C34 /
 * #2B313A, text #E4E7EB / #C3C9D1, muted #8E98A4, faint #4A525D, accent
 * #F2A33A, good #7FDBA9, warn #E9B949, bad #F59A8F, info #6CB4EE; keywords
 * #C792EA as in the approved design. Every token colour reads at 4.5:1 or
 * more on the ground.
 */
export const studioTheme = {
  base: 'vs-dark',
  inherit: true,
  rules: [
    { token: '', foreground: 'C3C9D1' },
    { token: 'keyword', foreground: 'C792EA' },
    { token: 'builtin', foreground: '6CB4EE' },
    { token: 'builtin.sailing', foreground: '6CB4EE', fontStyle: 'bold' },
    { token: 'predicate', foreground: 'E9B949', fontStyle: 'bold' },
    { token: 'identifier', foreground: 'E4E7EB' },
    { token: 'field', foreground: 'C3C9D1' },
    { token: 'operator', foreground: '8E98A4' },
    { token: 'delimiter', foreground: '8E98A4' },
    { token: 'string', foreground: '7FDBA9' },
    { token: 'string.escape', foreground: '7FDBA9', fontStyle: 'bold' },
    { token: 'string.invalid', foreground: 'F59A8F', fontStyle: 'italic' },
    { token: 'comment', foreground: '8A94A0', fontStyle: 'italic' },
    { token: 'number', foreground: 'F2A33A' },
    { token: 'invalid', foreground: 'F59A8F', fontStyle: 'underline' },
    { token: 'tag.jsx', foreground: '6CB4EE' },
    { token: 'delimiter.jsx', foreground: '8E98A4' },
    { token: 'attribute.name', foreground: 'C792EA' },
    { token: 'attribute.value', foreground: '7FDBA9' },
    { token: 'jsx.text', foreground: 'E4E7EB' },
  ],
  colors: {
    'editor.background': '#111418',
    'editor.foreground': '#C3C9D1',
    'editor.lineHighlightBackground': '#161A20',
    'editor.lineHighlightBorder': '#00000000',
    'editorLineNumber.foreground': '#4A525D',
    'editorLineNumber.activeForeground': '#8E98A4',
    'editor.selectionBackground': '#2F4A66',
    'editor.inactiveSelectionBackground': '#262C35',
    'editor.selectionHighlightBackground': '#2A313B',
    'editorIndentGuide.background1': '#262C34',
    'editorIndentGuide.activeBackground1': '#353C47',
    'editorGutter.background': '#111418',
    'editorCursor.foreground': '#F2A33A',
    'editorWidget.background': '#1B2027',
    'editorWidget.border': '#353C47',
    'editorHoverWidget.background': '#1B2027',
    'editorHoverWidget.border': '#353C47',
    'editorSuggestWidget.background': '#1B2027',
    'editorSuggestWidget.border': '#353C47',
    'editorSuggestWidget.selectedBackground': '#2A313B',
    'scrollbarSlider.background': '#2F364080',
    'scrollbarSlider.hoverBackground': '#353C47',
    'scrollbarSlider.activeBackground': '#4A525D',
    'editorBracketMatch.background': '#2A313B',
    'editorBracketMatch.border': '#4A525D',
    'editorError.foreground': '#F59A8F',
    'editorWarning.foreground': '#E9B949',
    'editorInfo.foreground': '#6CB4EE',
  },
};

/** The Monaco language for a project path. */
export function languageFor(path) {
  const m = /\.([A-Za-z0-9]+)$/.exec(path);
  switch (m ? m[1].toLowerCase() : '') {
    case 'expr': return EXPR;
    case 'json': return 'json';
    case 'jsx': case 'js': case 'mjs': case 'cjs': return 'javascript';
    case 'ts': case 'tsx': return 'typescript';
    case 'css': return 'css';
    case 'svg': case 'html': case 'htm': return 'html';
    case 'md': return 'markdown';
    default: return 'plaintext';
  }
}

/** A model's URI for a project path. */
export const modelUri = (path) => `file:///app/${path}`;

/** Which of the studio's JSON schemas validates a path: 'wire-schema', 'samples' or null. */
export function schemaFor(path) {
  if (/^engine\/[^/]+\.schema\.json$/.test(path)) return 'wire-schema';
  if (path === 'engine/samples.json') return 'samples';
  return null;
}
export const SCHEMAS = ['wire-schema', 'samples'];

// ------------------------------------------------------------- markers ----

/** Monaco's MarkerSeverity values (stable across versions). */
export const SEVERITY = { error: 8, warning: 4, info: 2, hint: 1 };

/** The UTF-16 column (Monaco's) of a 1-based column counted in characters (the compiler's). */
export function charColToUtf16(lineText, col) {
  let units = 0; let chars = 1;
  for (const ch of lineText) {
    if (chars >= col) break;
    units += ch.length; chars++;
  }
  return units + 1 + Math.max(0, col - chars);
}

/** 1-based { line, col } (UTF-16, Monaco's) of a UTF-8 byte offset in text. */
export function byteOffsetToLineCol(text, offset) {
  let bytes = 0; let line = 1; let col = 1;
  for (const ch of text) {
    if (bytes >= offset) break;
    const cp = ch.codePointAt(0);
    bytes += cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
    if (ch === '\n') { line++; col = 1; } else col += ch.length;
  }
  return { line, col };
}

/**
 * A compiler diagnostic (`langc check --json`, SPEC §5: {rule, severity, span:
 * {start: {line, col, offset}, end}, message, help}) → the editor's marker
 * {startLine, startCol, endLine, endCol, message, severity, rule} (1-based,
 * UTF-16 columns). A span may also be byte offsets ({start: n, end: n}, as a
 * sample run reports a run-time error), which needs the text. No span (an
 * interface or schema error): the first line.
 */
export function diagnosticToMarker(d, text = null) {
  const lines = text === null ? null : text.split('\n');
  const at = (p) => {
    if (typeof p === 'number') {
      if (text === null) return { line: 1, col: 1 };
      return byteOffsetToLineCol(text, p);
    }
    const line = Math.max(1, p?.line ?? 1);
    const col = Math.max(1, p?.col ?? 1);
    return { line, col: lines ? charColToUtf16(lines[line - 1] ?? '', col) : col };
  };
  const s = d.span ? at(d.span.start) : { line: 1, col: 1 };
  const e = d.span ? at(d.span.end ?? d.span.start) : { line: 1, col: lines ? (lines[0] ?? '').length + 1 : 1 };
  const message = (d.message ?? (d.error ? `run-time error ${d.error}` : '')) + (d.help ? `\nhelp: ${d.help}` : '');
  const severity = SEVERITY[d.severity] ? d.severity : 'error'; // a run-time error has no severity
  return { startLine: s.line, startCol: s.col, endLine: e.line, endCol: e.col, message, severity, rule: d.rule ?? d.error ?? '' };
}

/** The editor's marker → Monaco's IMarkerData. An empty range gets one character, so it shows. */
export function toMonacoMarker(m) {
  const empty = m.endLine === m.startLine && m.endCol <= m.startCol;
  return {
    startLineNumber: m.startLine,
    startColumn: m.startCol,
    endLineNumber: empty ? m.startLine : m.endLine,
    endColumn: empty ? m.startCol + 1 : m.endCol,
    message: m.message,
    severity: SEVERITY[m.severity] ?? SEVERITY.error,
    code: m.rule || undefined,
    source: m.source ?? 'langc',
  };
}
