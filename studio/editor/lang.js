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
export const THEME = 'studio-light';

/** Words the parser treats specially (syntax.rs; `if`/`else` per SPEC §2.1). */
export const KEYWORDS = ['let', 'in', 'nil', 'true', 'false', 'and', 'or', 'not', 'if', 'else'];

/**
 * expr's builtins the language accepts (lang/SPEC.md §2.3; the test checks
 * this list against the spec). Some aren't implemented yet (the expr-engine
 * skill lists which); they are still the language's names.
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
 * The studio's light theme (the page has no dark mode: `color-scheme: light`).
 * Colours from index.html's tokens: ink #15191E, ink-2 #34404D, muted
 * #5A626B, accent #C2410C / #9A3412, info #1E5BB8 / #1E4F9C, danger #A3201A,
 * warn #8A5300, line #D5DAD3, raised #FAFBF8, info-bg #E3ECF9.
 */
export const studioTheme = {
  base: 'vs',
  inherit: true,
  rules: [
    { token: 'keyword', foreground: '1E4F9C', fontStyle: 'bold' },
    { token: 'builtin', foreground: 'C2410C' },
    { token: 'builtin.sailing', foreground: '9A3412', fontStyle: 'bold' },
    { token: 'predicate', foreground: '8A5300', fontStyle: 'bold' },
    { token: 'identifier', foreground: '15191E' },
    { token: 'field', foreground: '34404D' },
    { token: 'operator', foreground: '5A626B' },
    { token: 'delimiter', foreground: '5A626B' },
    { token: 'string', foreground: '2E6B30' },
    { token: 'string.escape', foreground: '2E6B30', fontStyle: 'bold' },
    { token: 'string.invalid', foreground: 'A3201A', fontStyle: 'italic' },
    { token: 'comment', foreground: '6E767E', fontStyle: 'italic' },
    { token: 'number', foreground: '1E5BB8' },
    { token: 'invalid', foreground: 'A3201A', fontStyle: 'underline' },
    { token: 'tag.jsx', foreground: '1E4F9C' },
    { token: 'delimiter.jsx', foreground: '5A626B' },
    { token: 'attribute.name', foreground: '9A3412' },
    { token: 'attribute.value', foreground: '2E6B30' },
    { token: 'jsx.text', foreground: '15191E' },
  ],
  colors: {
    'editor.background': '#FFFFFF',
    'editor.foreground': '#15191E',
    'editor.lineHighlightBackground': '#F4F6F2',
    'editor.lineHighlightBorder': '#00000000',
    'editorLineNumber.foreground': '#8A9199',
    'editorLineNumber.activeForeground': '#34404D',
    'editor.selectionBackground': '#C9DAF3',
    'editor.inactiveSelectionBackground': '#E3ECF9',
    'editorIndentGuide.background1': '#E4E8E2',
    'editorGutter.background': '#FFFFFF',
    'editorCursor.foreground': '#C2410C',
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
