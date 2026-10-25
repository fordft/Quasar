//! The page-style validator. A model proposes CSS; only an allow-list of
//! plain style rules survives, and the output is re-serialized from the
//! admitted tokens, so no input text is ever forwarded. Anything that could
//! load, inject or smuggle (url(), image-set(), attr(), @import, @font-face,
//! escapes, comments, unbalanced input) rejects the whole sheet, because the
//! only way to produce it is to ignore the instructions — usually because the
//! page told the model to.
use std::fmt::Write as _;

use cssparser::{serialize_identifier, serialize_string, ParseError, Parser, ParserInput, Token};

/// Largest proposal accepted, in bytes.
pub const MAX_INPUT_BYTES: usize = 24 * 1024;
/// Largest stylesheet produced, in bytes.
pub const MAX_OUTPUT_BYTES: usize = 20 * 1024;
/// Most style rules one style may hold.
pub const MAX_RULES: usize = 200;
/// Deepest nesting of blocks and functions.
pub const MAX_DEPTH: u8 = 4;
/// Most tokens read from one proposal.
const MAX_TOKENS: usize = 8 * 1024;
/// Most selectors in one rule's list.
const MAX_SELECTORS: usize = 16;
/// Most compound selectors in one complex selector.
const MAX_COMPOUNDS: usize = 16;
/// Most declarations in one rule.
const MAX_DECLARATIONS: usize = 64;
/// Most value tokens in one declaration.
const MAX_VALUE_TOKENS: usize = 32;

/// Why a whole proposal was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rejection {
    /// Larger than [`MAX_INPUT_BYTES`] or produced more than the output bound.
    TooLarge,
    /// A construct that can load, inject or smuggle content.
    Forbidden,
    /// Unbalanced or unreadable CSS, or too deeply nested.
    Malformed,
    /// Nothing passed the allow-list.
    Empty,
}

/// An admitted stylesheet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Validated {
    /// Canonical CSS written from the admitted tokens only.
    pub css: String,
    /// Style rules kept.
    pub rules: u16,
    /// Rules and declarations left out because they were not allowed.
    pub dropped: u16,
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Ident(String),
    AtKeyword(String),
    /// A hash that is a valid identifier (`#id`), and its text.
    IdHash(String),
    /// A hash that is not a valid identifier (`#0af`).
    Hash(String),
    Str(String),
    Delim(char),
    Number {
        value: f32,
        int: Option<i32>,
    },
    Percentage {
        value: f32,
        int: Option<i32>,
    },
    Dimension {
        value: f32,
        int: Option<i32>,
        unit: String,
    },
    Space,
    Colon,
    Semicolon,
    Comma,
    Match(&'static str),
    Function(String, Vec<Node>),
    Paren(Vec<Node>),
    Square(Vec<Node>),
    Curly(Vec<Node>),
}

/// Validates and re-serializes a proposed stylesheet.
pub fn validate(input: &str) -> Result<Validated, Rejection> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(Rejection::TooLarge);
    }
    if input.contains('\\')
        || input
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r'))
    {
        return Err(Rejection::Forbidden);
    }
    let mut source = ParserInput::new(input);
    let mut parser = Parser::new(&mut source);
    let mut budget = MAX_TOKENS;
    let mut failure = None;
    let nodes = tree(&mut parser, 0, &mut budget, &mut failure).map_err(|_| Rejection::Malformed);
    if let Some(failure) = failure {
        return Err(failure);
    }
    let nodes = nodes?;
    let mut writer = Writer::default();
    rules(&nodes, 0, &mut writer)?;
    if writer.rules == 0 {
        return Err(Rejection::Empty);
    }
    if writer.css.len() > MAX_OUTPUT_BYTES {
        return Err(Rejection::TooLarge);
    }
    Ok(Validated {
        css: writer.css,
        rules: writer.rules,
        dropped: writer.dropped,
    })
}

fn tree<'i>(
    parser: &mut Parser<'i, '_>,
    depth: u8,
    budget: &mut usize,
    failure: &mut Option<Rejection>,
) -> Result<Vec<Node>, ParseError<'i, ()>> {
    let mut nodes = Vec::new();
    loop {
        let Ok(token) = parser.next_including_whitespace_and_comments() else {
            return Ok(nodes);
        };
        let token = token.clone();
        if *budget == 0 {
            *failure = Some(Rejection::TooLarge);
            return Err(parser.new_custom_error(()));
        }
        *budget -= 1;
        let nested = matches!(
            token,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        );
        if nested && depth >= MAX_DEPTH * 2 {
            *failure = Some(Rejection::Malformed);
            return Err(parser.new_custom_error(()));
        }
        let node = match token {
            Token::Ident(value) => Node::Ident(value.to_string()),
            Token::AtKeyword(value) => Node::AtKeyword(value.to_ascii_lowercase()),
            Token::IDHash(value) => Node::IdHash(value.to_string()),
            Token::Hash(value) => Node::Hash(value.to_string()),
            Token::QuotedString(value) => Node::Str(value.to_string()),
            Token::Delim(value) => Node::Delim(value),
            Token::Number {
                value, int_value, ..
            } => Node::Number {
                value,
                int: int_value,
            },
            Token::Percentage {
                unit_value,
                int_value,
                ..
            } => Node::Percentage {
                value: unit_value * 100.0,
                int: int_value,
            },
            Token::Dimension {
                value,
                int_value,
                unit,
                ..
            } => Node::Dimension {
                value,
                int: int_value,
                unit: unit.to_ascii_lowercase(),
            },
            Token::WhiteSpace(_) => Node::Space,
            Token::Colon => Node::Colon,
            Token::Semicolon => Node::Semicolon,
            Token::Comma => Node::Comma,
            Token::IncludeMatch => Node::Match("~="),
            Token::DashMatch => Node::Match("|="),
            Token::PrefixMatch => Node::Match("^="),
            Token::SuffixMatch => Node::Match("$="),
            Token::SubstringMatch => Node::Match("*="),
            Token::Function(name) => {
                let name = name.to_ascii_lowercase();
                if forbidden_function(&name) {
                    *failure = Some(Rejection::Forbidden);
                    return Err(parser.new_custom_error(()));
                }
                let inner = parser.parse_nested_block(|p| tree(p, depth + 1, budget, failure))?;
                Node::Function(name, inner)
            }
            Token::ParenthesisBlock => {
                Node::Paren(parser.parse_nested_block(|p| tree(p, depth + 1, budget, failure))?)
            }
            Token::SquareBracketBlock => {
                Node::Square(parser.parse_nested_block(|p| tree(p, depth + 1, budget, failure))?)
            }
            Token::CurlyBracketBlock => {
                Node::Curly(parser.parse_nested_block(|p| tree(p, depth + 1, budget, failure))?)
            }
            Token::Comment(_) | Token::UnquotedUrl(_) | Token::CDO | Token::CDC => {
                *failure = Some(Rejection::Forbidden);
                return Err(parser.new_custom_error(()));
            }
            Token::BadUrl(_)
            | Token::BadString(_)
            | Token::CloseParenthesis
            | Token::CloseSquareBracket
            | Token::CloseCurlyBracket => {
                *failure = Some(Rejection::Malformed);
                return Err(parser.new_custom_error(()));
            }
        };
        nodes.push(node);
    }
}

/// Parses one compound selector (no combinators) with the sheet validator's
/// own grammar and returns its canonical text.
pub(crate) fn compound_selector(input: &str) -> Option<String> {
    if input.is_empty()
        || input.len() > 256
        || input.trim() != input
        || input.contains('\\')
        || input.chars().any(char::is_control)
    {
        return None;
    }
    let mut source = ParserInput::new(input);
    let mut parser = Parser::new(&mut source);
    let mut budget = 64;
    let mut failure = None;
    let nodes = tree(&mut parser, 0, &mut budget, &mut failure).ok()?;
    if failure.is_some() {
        return None;
    }
    let refs: Vec<&Node> = nodes.iter().collect();
    complex(&refs, false)
}

fn forbidden_function(name: &str) -> bool {
    let bare = name.trim_start_matches('-');
    let bare = ["webkit-", "moz-", "ms-", "o-"]
        .iter()
        .find_map(|prefix| bare.strip_prefix(prefix))
        .unwrap_or(bare);
    matches!(
        bare,
        "url"
            | "src"
            | "image"
            | "image-set"
            | "cross-fade"
            | "element"
            | "paint"
            | "expression"
            | "attr"
            | "env"
            | "binding"
    )
}

#[derive(Default)]
struct Writer {
    css: String,
    rules: u16,
    dropped: u16,
}

impl Writer {
    fn drop_one(&mut self) {
        self.dropped = self.dropped.saturating_add(1);
    }
}

/// A list of rules: style rules and `@media` blocks.
fn rules(nodes: &[Node], depth: u8, out: &mut Writer) -> Result<(), Rejection> {
    let mut prelude: Vec<&Node> = Vec::new();
    for node in nodes {
        match node {
            Node::Curly(block) => {
                let head = std::mem::take(&mut prelude);
                rule(&head, block, depth, out)?;
            }
            Node::Semicolon if prelude.iter().all(|n| matches!(n, Node::Space)) => {
                prelude.clear();
            }
            Node::Semicolon => {
                // A statement at-rule such as @import, or stray tokens.
                if matches!(first_meaningful(&prelude), Some(Node::AtKeyword(_))) {
                    return Err(Rejection::Forbidden);
                }
                out.drop_one();
                prelude.clear();
            }
            other => prelude.push(other),
        }
    }
    match first_meaningful(&prelude) {
        Some(Node::AtKeyword(_)) => Err(Rejection::Forbidden),
        Some(_) => Err(Rejection::Malformed),
        None => Ok(()),
    }
}

fn first_meaningful<'a>(nodes: &[&'a Node]) -> Option<&'a Node> {
    nodes
        .iter()
        .copied()
        .find(|node| !matches!(node, Node::Space))
}

fn rule(prelude: &[&Node], block: &[Node], depth: u8, out: &mut Writer) -> Result<(), Rejection> {
    if depth >= MAX_DEPTH {
        return Err(Rejection::Malformed);
    }
    if let Some(Node::AtKeyword(name)) = first_meaningful(prelude) {
        if name != "media" {
            return Err(Rejection::Forbidden);
        }
        let start = prelude
            .iter()
            .position(|n| matches!(n, Node::AtKeyword(_)))
            .map_or(0, |at| at + 1);
        let Some(query) = media_query(&prelude[start..]) else {
            out.drop_one();
            return Ok(());
        };
        let mut inner = Writer::default();
        rules(block, depth + 1, &mut inner)?;
        out.dropped = out.dropped.saturating_add(inner.dropped);
        if inner.rules > 0 {
            let _ = write!(out.css, "@media {query}{{\n{}}}\n", inner.css);
            out.rules = out.rules.saturating_add(inner.rules);
        }
        return Ok(());
    }
    if usize::from(out.rules) >= MAX_RULES {
        out.drop_one();
        return Ok(());
    }
    let mut declarations = Vec::new();
    for declaration in block.split(|node| matches!(node, Node::Semicolon)) {
        if declaration.iter().all(|node| matches!(node, Node::Space)) {
            continue;
        }
        if declarations.len() >= MAX_DECLARATIONS {
            out.drop_one();
            continue;
        }
        match self::declaration(declaration)? {
            Some(text) => declarations.push(text),
            None => out.drop_one(),
        }
    }
    // Declarations are checked before the selector, so a forbidden one
    // rejects the sheet even under a selector that would be dropped.
    let Some(selectors) = selector_list(prelude) else {
        out.drop_one();
        return Ok(());
    };
    if declarations.is_empty() {
        out.drop_one();
        return Ok(());
    }
    let _ = writeln!(out.css, "{selectors}{{{}}}", declarations.join(";"));
    out.rules = out.rules.saturating_add(1);
    Ok(())
}

// ---------------------------------------------------------------- @media

fn media_query(nodes: &[&Node]) -> Option<String> {
    let words: Vec<&Node> = nodes
        .iter()
        .copied()
        .filter(|n| !matches!(n, Node::Space))
        .collect();
    let mut parts = Vec::new();
    let mut expect_condition = true;
    for (index, node) in words.iter().enumerate() {
        match node {
            Node::Ident(word) if index == 0 && matches!(lower(word).as_str(), "screen" | "all") => {
                parts.push(lower(word));
                expect_condition = false;
            }
            Node::Ident(word) if !expect_condition && lower(word) == "and" => {
                parts.push("and".to_owned());
                expect_condition = true;
            }
            Node::Paren(inner) if expect_condition => {
                parts.push(media_feature(inner)?);
                expect_condition = false;
            }
            _ => return None,
        }
    }
    (!parts.is_empty() && !expect_condition).then(|| parts.join(" "))
}

fn media_feature(nodes: &[Node]) -> Option<String> {
    let words: Vec<&Node> = nodes.iter().filter(|n| !matches!(n, Node::Space)).collect();
    let [Node::Ident(feature), Node::Colon, value] = words.as_slice() else {
        return None;
    };
    let feature = lower(feature);
    let value = match (feature.as_str(), value) {
        ("prefers-color-scheme", Node::Ident(scheme))
            if matches!(lower(scheme).as_str(), "dark" | "light") =>
        {
            lower(scheme)
        }
        ("min-width" | "max-width", Node::Dimension { unit, .. })
            if matches!(unit.as_str(), "px" | "em" | "rem") =>
        {
            number(value)?
        }
        _ => return None,
    };
    Some(format!("({feature}: {value})"))
}

// ---------------------------------------------------------------- selectors

fn ident(value: &str) -> Option<String> {
    let mut text = String::new();
    serialize_identifier(value, &mut text).ok()?;
    Some(text)
}

fn lower(value: &str) -> String {
    value.to_ascii_lowercase()
}

fn plain_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

const PSEUDO_CLASSES: &[&str] = &[
    "hover",
    "focus",
    "focus-visible",
    "focus-within",
    "active",
    "link",
    "visited",
    "checked",
    "disabled",
    "enabled",
    "empty",
    "root",
    "first-child",
    "last-child",
    "only-child",
    "first-of-type",
    "last-of-type",
    "only-of-type",
];
const PSEUDO_ELEMENTS: &[&str] = &[
    "before",
    "after",
    "selection",
    "placeholder",
    "marker",
    "first-line",
    "first-letter",
];
const NTH: &[&str] = &[
    "nth-child",
    "nth-last-child",
    "nth-of-type",
    "nth-last-of-type",
];

fn selector_list(nodes: &[&Node]) -> Option<String> {
    let mut selectors = Vec::new();
    for group in nodes.split(|node| matches!(node, Node::Comma)) {
        if selectors.len() == MAX_SELECTORS {
            return None;
        }
        selectors.push(complex(group, true)?);
    }
    let list = selectors.join(", ");
    (list.len() <= 1024).then_some(list)
}

/// A complex selector; without `combinators`, one compound only.
fn complex(nodes: &[&Node], combinators: bool) -> Option<String> {
    let start = nodes.iter().position(|n| !matches!(n, Node::Space))?;
    let end = nodes.iter().rposition(|n| !matches!(n, Node::Space))? + 1;
    let nodes = &nodes[start..end];
    let mut output = String::new();
    let mut compound: Vec<&Node> = Vec::new();
    let mut compounds = 0;
    let mut pending: Option<&str> = None;
    let mut index = 0;
    while index < nodes.len() {
        let node = nodes[index];
        let combinator = match node {
            Node::Space => Some(" "),
            Node::Delim('>') => Some(" > "),
            Node::Delim('+') => Some(" + "),
            Node::Delim('~') => Some(" ~ "),
            _ => None,
        };
        match combinator {
            Some(found) => {
                if !combinators {
                    return None;
                }
                if !compound.is_empty() {
                    output.push_str(&self::compound(&compound)?);
                    compound.clear();
                    compounds += 1;
                    pending = Some(found);
                } else if found != " " {
                    // A second explicit combinator in a row, or a leading one.
                    if pending != Some(" ") {
                        return None;
                    }
                    pending = Some(found);
                }
            }
            None => {
                if let Some(joiner) = pending.take() {
                    output.push_str(joiner);
                }
                compound.push(node);
            }
        }
        index += 1;
    }
    if compound.is_empty() {
        return None;
    }
    output.push_str(&self::compound(&compound)?);
    compounds += 1;
    (compounds <= MAX_COMPOUNDS).then_some(output)
}

fn compound(nodes: &[&Node]) -> Option<String> {
    let mut output = String::new();
    let mut index = 0;
    let mut element_done = false;
    match nodes.first()? {
        Node::Ident(name) if plain_name(name) => {
            output.push_str(&ident(&lower(name))?);
            index = 1;
        }
        Node::Delim('*') => {
            output.push('*');
            index = 1;
        }
        _ => {}
    }
    while index < nodes.len() {
        if element_done {
            return None;
        }
        match nodes[index] {
            Node::IdHash(name) if plain_name(name) => {
                output.push('#');
                output.push_str(&ident(name)?);
            }
            Node::Delim('.') => {
                index += 1;
                let Some(Node::Ident(name)) = nodes.get(index) else {
                    return None;
                };
                if !plain_name(name) {
                    return None;
                }
                output.push('.');
                output.push_str(&ident(name)?);
            }
            Node::Square(inner) => output.push_str(&attribute(inner)?),
            Node::Colon => {
                index += 1;
                match nodes.get(index)? {
                    Node::Colon => {
                        index += 1;
                        let Node::Ident(name) = nodes.get(index)? else {
                            return None;
                        };
                        let name = lower(name);
                        if !PSEUDO_ELEMENTS.contains(&name.as_str()) {
                            return None;
                        }
                        output.push_str("::");
                        output.push_str(&name);
                        element_done = true;
                    }
                    Node::Ident(name) => {
                        let name = lower(name);
                        if !PSEUDO_CLASSES.contains(&name.as_str()) {
                            return None;
                        }
                        output.push(':');
                        output.push_str(&name);
                    }
                    Node::Function(name, inner) if name == "not" => {
                        let inner: Vec<&Node> = inner.iter().collect();
                        let mut alternatives = Vec::new();
                        for group in inner.split(|n| matches!(n, Node::Comma)) {
                            alternatives.push(complex(group, false)?);
                        }
                        if alternatives.len() > 4 {
                            return None;
                        }
                        let _ = write!(output, ":not({})", alternatives.join(", "));
                    }
                    Node::Function(name, inner) if NTH.contains(&name.as_str()) => {
                        let _ = write!(output, ":{name}({})", nth(inner)?);
                    }
                    _ => return None,
                }
            }
            _ => return None,
        }
        index += 1;
    }
    (!output.is_empty()).then_some(output)
}

fn attribute(nodes: &[Node]) -> Option<String> {
    let words: Vec<&Node> = nodes.iter().filter(|n| !matches!(n, Node::Space)).collect();
    let (name, rest) = match words.as_slice() {
        [Node::Ident(name), rest @ ..] if plain_name(name) => (lower(name), rest),
        _ => return None,
    };
    let name = ident(&name)?;
    let (operator, value, flag) = match rest {
        [] => return Some(format!("[{name}]")),
        [operator, value] => (operator, value, None),
        [operator, value, Node::Ident(flag)] if matches!(lower(flag).as_str(), "i" | "s") => {
            (operator, value, Some(lower(flag)))
        }
        _ => return None,
    };
    let operator = match operator {
        Node::Delim('=') => "=",
        Node::Match(operator) => operator,
        _ => return None,
    };
    let value = match value {
        Node::Ident(value) | Node::Str(value) if value.chars().count() <= 256 => value,
        _ => return None,
    };
    let mut quoted = String::new();
    serialize_string(value, &mut quoted).ok()?;
    Some(match flag {
        Some(flag) => format!("[{name}{operator}{quoted} {flag}]"),
        None => format!("[{name}{operator}{quoted}]"),
    })
}

fn nth(nodes: &[Node]) -> Option<String> {
    let words: Vec<&Node> = nodes.iter().filter(|n| !matches!(n, Node::Space)).collect();
    match words.as_slice() {
        [Node::Ident(word)] if matches!(lower(word).as_str(), "odd" | "even") => Some(lower(word)),
        [Node::Number { int: Some(b), .. }] if b.unsigned_abs() <= 1000 => Some(b.to_string()),
        [Node::Dimension {
            int: Some(a), unit, ..
        }] if unit == "n" && a.unsigned_abs() <= 1000 => Some(format!("{a}n")),
        [Node::Dimension {
            int: Some(a), unit, ..
        }, Node::Number { int: Some(b), .. }]
            if unit == "n" && a.unsigned_abs() <= 1000 && b.unsigned_abs() <= 1000 =>
        {
            Some(format!("{a}n{b:+}"))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- declarations

#[derive(Clone, Copy)]
struct Grammar {
    /// Lengths, percentages and math functions.
    length: bool,
    /// Plain numbers.
    number: bool,
    color: bool,
    keywords: &'static [&'static str],
    /// Font family names: any identifier or string.
    family: bool,
    commas: bool,
    slash: bool,
    filters: bool,
    /// Grid tracks: `fr` and `repeat()`, `minmax()`, `fit-content()`.
    tracks: bool,
    /// Author names, such as counter names.
    names: bool,
    max_values: usize,
}

const NONE: Grammar = Grammar {
    length: false,
    number: false,
    color: false,
    keywords: &[],
    family: false,
    commas: false,
    slash: false,
    filters: false,
    tracks: false,
    names: false,
    max_values: 1,
};

const ALIGNMENT: &[&str] = &[
    "normal",
    "stretch",
    "center",
    "start",
    "end",
    "flex-start",
    "flex-end",
    "self-start",
    "self-end",
    "baseline",
    "first",
    "last",
    "left",
    "right",
    "space-between",
    "space-around",
    "space-evenly",
];

/// What `content` may hold: nothing, a counter, or a separator. Free text is
/// how a page would make the model write words onto a site.
const SEPARATORS: &[&str] = &[
    "", " ", "·", " · ", "•", " • ", "|", " | ", "—", " — ", "–", " – ", "/", " / ", ",", ", ",
    ".", ". ", ":", ": ", "#", "(", ")", "→", " → ",
];

const BORDER_STYLES: &[&str] = &[
    "none", "hidden", "solid", "dashed", "dotted", "double", "groove", "ridge", "inset", "outset",
    "thin", "medium", "thick",
];

fn grammar(property: &str) -> Option<Grammar> {
    let color = Grammar {
        color: true,
        ..NONE
    };
    let length = |keywords: &'static [&'static str], max_values: usize| Grammar {
        length: true,
        keywords,
        max_values,
        ..NONE
    };
    let keywords = |keywords: &'static [&'static str]| Grammar { keywords, ..NONE };
    let border = Grammar {
        length: true,
        color: true,
        keywords: BORDER_STYLES,
        max_values: 3,
        ..NONE
    };
    Some(match property {
        "color"
        | "background-color"
        | "outline-color"
        | "text-decoration-color"
        | "caret-color"
        | "accent-color"
        | "column-rule-color"
        | "border-top-color"
        | "border-right-color"
        | "border-bottom-color"
        | "border-left-color" => color,
        "border-color" => Grammar {
            max_values: 4,
            ..color
        },
        "background" => Grammar {
            keywords: &["none", "transparent"],
            ..color
        },
        "font-family" => Grammar {
            family: true,
            commas: true,
            max_values: 8,
            ..NONE
        },
        "font-size" => length(
            &[
                "xx-small",
                "x-small",
                "small",
                "medium",
                "large",
                "x-large",
                "xx-large",
                "xxx-large",
                "smaller",
                "larger",
            ],
            1,
        ),
        "font-weight" => Grammar {
            number: true,
            keywords: &["normal", "bold", "bolder", "lighter"],
            ..NONE
        },
        "font-style" => keywords(&["normal", "italic", "oblique"]),
        "line-height" => Grammar {
            number: true,
            ..length(&["normal"], 1)
        },
        "letter-spacing" | "word-spacing" => length(&["normal"], 1),
        "text-align" => keywords(&["left", "right", "center", "justify", "start", "end"]),
        "text-transform" => keywords(&["none", "capitalize", "uppercase", "lowercase"]),
        "text-decoration" | "text-decoration-line" | "text-decoration-style" => Grammar {
            length: true,
            color: true,
            keywords: &[
                "none",
                "underline",
                "overline",
                "line-through",
                "solid",
                "double",
                "dotted",
                "dashed",
                "wavy",
            ],
            max_values: 4,
            ..NONE
        },
        "margin" => length(&["auto"], 4),
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "margin-inline"
        | "margin-block" => length(&["auto"], 2),
        "padding" => length(&[], 4),
        "padding-top" | "padding-right" | "padding-bottom" | "padding-left" | "padding-inline"
        | "padding-block" => length(&[], 2),
        "gap" => length(&["normal"], 2),
        "row-gap" | "column-gap" => length(&["normal"], 1),
        "width" | "min-width" | "max-width" => length(
            &["auto", "none", "min-content", "max-content", "fit-content"],
            1,
        ),
        "min-height" => length(&["auto", "min-content", "max-content", "fit-content"], 1),
        "display" => keywords(&[
            "none",
            "block",
            "inline",
            "inline-block",
            "flex",
            "inline-flex",
            "grid",
            "inline-grid",
            "flow-root",
            "contents",
            "list-item",
            "table",
            "table-row",
            "table-cell",
        ]),
        "box-sizing" => keywords(&["border-box", "content-box"]),
        "flex-direction" => keywords(&["row", "row-reverse", "column", "column-reverse"]),
        "flex-wrap" => keywords(&["nowrap", "wrap", "wrap-reverse"]),
        "flex-flow" => Grammar {
            max_values: 2,
            ..keywords(&[
                "row",
                "row-reverse",
                "column",
                "column-reverse",
                "nowrap",
                "wrap",
                "wrap-reverse",
            ])
        },
        "flex" => Grammar {
            number: true,
            ..length(&["none", "auto", "content"], 3)
        },
        "flex-grow" | "flex-shrink" | "order" => Grammar {
            number: true,
            ..NONE
        },
        "flex-basis" => length(&["auto", "content", "min-content", "max-content"], 1),
        "align-items" | "align-self" | "align-content" | "justify-content" | "justify-items"
        | "justify-self" | "place-items" | "place-content" | "place-self" => Grammar {
            max_values: 2,
            ..keywords(ALIGNMENT)
        },
        "grid-template-columns" | "grid-template-rows" | "grid-auto-columns" | "grid-auto-rows" => {
            Grammar {
                tracks: true,
                ..length(&["none", "auto", "min-content", "max-content"], 12)
            }
        }
        "grid-auto-flow" => Grammar {
            max_values: 2,
            ..keywords(&["row", "column", "dense"])
        },
        "grid-column" | "grid-row" => Grammar {
            number: true,
            slash: true,
            keywords: &["auto", "span"],
            max_values: 5,
            ..NONE
        },
        "vertical-align" => keywords(&[
            "baseline",
            "top",
            "middle",
            "bottom",
            "text-top",
            "text-bottom",
        ]),
        "list-style-type" | "list-style" => {
            keywords(&["none", "disc", "circle", "square", "decimal"])
        }
        "list-style-position" => keywords(&["inside", "outside"]),
        "white-space" => keywords(&["normal", "nowrap", "pre-line", "pre-wrap"]),
        "text-wrap" => keywords(&["wrap", "balance", "pretty"]),
        "text-underline-offset" => length(&["auto"], 1),
        "border-collapse" => keywords(&["collapse", "separate"]),
        "border-spacing" => length(&[], 2),
        "table-layout" => keywords(&["auto", "fixed"]),
        "counter-reset" | "counter-increment" => Grammar {
            number: true,
            names: true,
            keywords: &["none"],
            max_values: 4,
            ..NONE
        },
        "visibility" => keywords(&["visible", "hidden", "collapse"]),
        "opacity" => Grammar {
            number: true,
            length: true,
            ..NONE
        },
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" | "outline" => {
            border
        }
        "border-width" | "border-style" => Grammar {
            max_values: 4,
            ..border
        },
        "border-top-width"
        | "border-right-width"
        | "border-bottom-width"
        | "border-left-width"
        | "border-top-style"
        | "border-right-style"
        | "border-bottom-style"
        | "border-left-style"
        | "outline-width"
        | "outline-style" => Grammar {
            max_values: 1,
            ..border
        },
        "outline-offset" => length(&[], 1),
        "border-radius" => Grammar {
            slash: true,
            ..length(&[], 9)
        },
        "border-top-left-radius"
        | "border-top-right-radius"
        | "border-bottom-left-radius"
        | "border-bottom-right-radius" => length(&[], 2),
        "box-shadow" | "text-shadow" => Grammar {
            length: true,
            color: true,
            commas: true,
            keywords: &["none", "inset"],
            max_values: 24,
            ..NONE
        },
        "filter" => Grammar {
            filters: true,
            keywords: &["none"],
            max_values: 8,
            ..NONE
        },
        "color-scheme" => Grammar {
            max_values: 2,
            ..keywords(&["normal", "light", "dark", "only"])
        },
        _ => return None,
    })
}

/// Properties whose mere presence means the proposal ignored its limits in
/// a way that could load or inject content.
fn forbidden_property(property: &str) -> bool {
    let bare = property.trim_start_matches('-');
    bare.contains("binding") || bare.ends_with("behavior") || matches!(bare, "src" | "behaviour")
}

const GLOBAL_KEYWORDS: &[&str] = &["inherit", "initial", "unset", "revert", "revert-layer"];

/// One declaration: `Ok(None)` drops it, `Err` rejects the sheet.
fn declaration(nodes: &[Node]) -> Result<Option<String>, Rejection> {
    let mut words: Vec<&Node> = nodes.iter().collect();
    while matches!(words.first(), Some(Node::Space)) {
        words.remove(0);
    }
    let (Some(Node::Ident(name)), rest) = (words.first().copied(), &words[1.min(words.len())..])
    else {
        return Ok(None);
    };
    let property = lower(name);
    if forbidden_property(&property) {
        return Err(Rejection::Forbidden);
    }
    let rest: Vec<&Node> = rest
        .iter()
        .copied()
        .skip_while(|n| matches!(n, Node::Space))
        .collect();
    let Some((Node::Colon, value)) = rest.split_first().map(|(c, v)| (*c, v)) else {
        return Ok(None);
    };
    let mut value: Vec<&Node> = value.to_vec();
    while matches!(value.last(), Some(Node::Space)) {
        value.pop();
    }
    let mut important = false;
    if let [.., Node::Delim('!'), Node::Ident(word)] = value.as_slice() {
        if lower(word) == "important" {
            important = true;
            value.truncate(value.len() - 2);
            while matches!(value.last(), Some(Node::Space)) {
                value.pop();
            }
        }
    }
    let text = if property == "content" {
        content(&value)?
    } else {
        let Some(grammar) = grammar(&property) else {
            return Ok(None);
        };
        let Some(text) = value_text(&value, grammar) else {
            return Ok(None);
        };
        text
    };
    Ok(Some(if important {
        format!("{property}:{text}!important")
    } else {
        format!("{property}:{text}")
    }))
}

/// A `content` value: counters and separators only; anything else rejects
/// the sheet, as only a page could have asked for it.
fn content(nodes: &[&Node]) -> Result<String, Rejection> {
    let words: Vec<&Node> = nodes
        .iter()
        .copied()
        .filter(|n| !matches!(n, Node::Space))
        .collect();
    if let [Node::Ident(word)] = words.as_slice() {
        if matches!(lower(word).as_str(), "none" | "normal") {
            return Ok(lower(word));
        }
    }
    if words.is_empty() || words.len() > 4 {
        return Err(Rejection::Forbidden);
    }
    let mut pieces = Vec::with_capacity(words.len());
    for word in words {
        pieces.push(match word {
            Node::Str(text) if SEPARATORS.contains(&text.as_str()) => {
                let mut quoted = String::new();
                serialize_string(text, &mut quoted).map_err(|_| Rejection::Forbidden)?;
                quoted
            }
            Node::Function(name, inner) if name == "counter" => {
                let args: Vec<&Node> = inner.iter().filter(|n| !matches!(n, Node::Space)).collect();
                match args.as_slice() {
                    [Node::Ident(counter)] if plain_name(counter) => format!(
                        "counter({})",
                        ident(&lower(counter)).ok_or(Rejection::Forbidden)?
                    ),
                    [Node::Ident(counter), Node::Comma, Node::Ident(style)]
                        if plain_name(counter)
                            && matches!(
                                lower(style).as_str(),
                                "decimal" | "decimal-leading-zero"
                            ) =>
                    {
                        format!(
                            "counter({}, {})",
                            ident(&lower(counter)).ok_or(Rejection::Forbidden)?,
                            lower(style)
                        )
                    }
                    _ => return Err(Rejection::Forbidden),
                }
            }
            _ => return Err(Rejection::Forbidden),
        });
    }
    Ok(pieces.join(" "))
}

fn value_text(nodes: &[&Node], grammar: Grammar) -> Option<String> {
    let words: Vec<&Node> = nodes
        .iter()
        .copied()
        .skip_while(|n| matches!(n, Node::Space))
        .collect();
    if let [Node::Ident(word)] = words.as_slice() {
        let word = lower(word);
        if GLOBAL_KEYWORDS.contains(&word.as_str()) {
            return Some(word);
        }
    }
    let mut output = String::new();
    let mut values = 0;
    let mut space = false;
    let mut tokens = 0;
    for node in words {
        tokens += 1;
        if tokens > MAX_VALUE_TOKENS {
            return None;
        }
        let piece = match node {
            Node::Space => {
                space = true;
                continue;
            }
            Node::Comma if grammar.commas => {
                output.push_str(", ");
                space = false;
                continue;
            }
            Node::Delim('/') if grammar.slash => "/".to_owned(),
            Node::Ident(word) => {
                let word = lower(word);
                if grammar.keywords.contains(&word.as_str()) || grammar.color && color_name(&word) {
                    word
                } else if (grammar.family || grammar.names) && plain_name(&word) {
                    ident(&word)?
                } else {
                    return None;
                }
            }
            Node::Str(family) if grammar.family && family.chars().count() <= 64 => {
                let mut quoted = String::new();
                serialize_string(family, &mut quoted).ok()?;
                quoted
            }
            Node::IdHash(hex) | Node::Hash(hex) if grammar.color => hex_color(hex)?,
            Node::Number { .. } if grammar.number || grammar.length => {
                let text = number(node)?;
                if !grammar.number && text != "0" {
                    return None;
                }
                text
            }
            Node::Dimension { unit, .. } if grammar.tracks && unit == "fr" => number(node)?,
            Node::Percentage { .. } | Node::Dimension { .. } if grammar.length => length(node)?,
            Node::Function(name, inner)
                if grammar.tracks
                    && matches!(name.as_str(), "repeat" | "minmax" | "fit-content") =>
            {
                tracks(name, inner, 0)?
            }
            Node::Function(name, inner) => function(name, inner, grammar, 0)?,
            _ => return None,
        };
        if space && !output.is_empty() && !output.ends_with(' ') {
            output.push(' ');
        }
        space = false;
        output.push_str(&piece);
        values += 1;
        if values > grammar.max_values {
            return None;
        }
    }
    (!output.is_empty()).then_some(output)
}

fn color_name(word: &str) -> bool {
    (3..=20).contains(&word.len()) && word.bytes().all(|b| b.is_ascii_lowercase())
}

fn hex_color(hex: &str) -> Option<String> {
    (matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("#{}", hex.to_ascii_lowercase()))
}

fn number(node: &Node) -> Option<String> {
    let (value, int, unit) = match node {
        Node::Number { value, int } => (*value, *int, ""),
        Node::Percentage { value, int } => (*value, *int, "%"),
        Node::Dimension { value, int, unit } => (*value, *int, unit.as_str()),
        _ => return None,
    };
    if !value.is_finite() || value.abs() > 10_000.0 {
        return None;
    }
    let digits = match int {
        Some(int) => int.to_string(),
        None => {
            let text = format!("{value:.4}");
            let text = text.trim_end_matches('0').trim_end_matches('.');
            if text == "-0" {
                "0".to_owned()
            } else {
                text.to_owned()
            }
        }
    };
    Some(format!("{digits}{unit}"))
}

const LENGTH_UNITS: &[&str] = &[
    "px", "em", "rem", "ex", "ch", "vw", "vh", "vmin", "vmax", "pt", "lh", "rlh",
];

fn length(node: &Node) -> Option<String> {
    match node {
        Node::Percentage { .. } => number(node),
        Node::Dimension { unit, .. } if LENGTH_UNITS.contains(&unit.as_str()) => number(node),
        _ => None,
    }
}

/// Grid track functions: `repeat(auto-fill, minmax(240px, 1fr))` and kin.
fn tracks(name: &str, inner: &[Node], depth: u8) -> Option<String> {
    if depth > 2 {
        return None;
    }
    let mut output = format!("{name}(");
    let mut space = false;
    for (count, node) in inner.iter().enumerate() {
        if count > MAX_VALUE_TOKENS {
            return None;
        }
        let piece = match node {
            Node::Space => {
                space = true;
                continue;
            }
            Node::Comma => {
                output.push_str(", ");
                space = false;
                continue;
            }
            Node::Number { int: Some(n), .. } if name == "repeat" && (1..=24).contains(n) => {
                n.to_string()
            }
            Node::Number { .. } => {
                let text = number(node)?;
                (text == "0").then_some(text)?
            }
            Node::Dimension { unit, .. } if unit == "fr" => number(node)?,
            Node::Percentage { .. } | Node::Dimension { .. } => length(node)?,
            Node::Ident(word)
                if matches!(
                    lower(word).as_str(),
                    "auto" | "auto-fill" | "auto-fit" | "min-content" | "max-content"
                ) =>
            {
                lower(word)
            }
            Node::Function(inner_name, nested)
                if matches!(inner_name.as_str(), "minmax" | "fit-content") =>
            {
                tracks(inner_name, nested, depth + 1)?
            }
            _ => return None,
        };
        if space && !output.ends_with('(') && !output.ends_with(' ') {
            output.push(' ');
        }
        space = false;
        output.push_str(&piece);
    }
    output.push(')');
    Some(output)
}

fn function(name: &str, inner: &[Node], grammar: Grammar, depth: u8) -> Option<String> {
    if depth > 2 {
        return None;
    }
    let math = matches!(name, "calc" | "min" | "max" | "clamp");
    let color = matches!(
        name,
        "rgb" | "rgba" | "hsl" | "hsla" | "hwb" | "lab" | "lch" | "oklab" | "oklch"
    );
    let filter = matches!(
        name,
        "invert"
            | "brightness"
            | "contrast"
            | "grayscale"
            | "sepia"
            | "saturate"
            | "hue-rotate"
            | "opacity"
            | "blur"
    );
    if !(math && grammar.length || color && grammar.color || filter && grammar.filters) {
        return None;
    }
    let mut output = format!("{name}(");
    let mut space = false;
    let mut count = 0;
    for node in inner {
        count += 1;
        if count > MAX_VALUE_TOKENS {
            return None;
        }
        let piece = match node {
            Node::Space => {
                space = true;
                continue;
            }
            Node::Comma if math || color => {
                output.push_str(", ");
                space = false;
                continue;
            }
            Node::Delim(op @ ('+' | '-' | '*' | '/')) if math => {
                output.push_str(&format!(" {op} "));
                space = false;
                continue;
            }
            Node::Delim('/') if color => {
                output.push_str(" / ");
                space = false;
                continue;
            }
            Node::Number { .. } => number(node)?,
            Node::Percentage { .. } => number(node)?,
            Node::Dimension { unit, .. } if matches!(unit.as_str(), "deg" | "turn") && !math => {
                number(node)?
            }
            Node::Dimension { .. } if math || name == "blur" => length(node)?,
            Node::Ident(word) if color && lower(word) == "none" => "none".to_owned(),
            Node::Function(inner_name, nested) if math => {
                function(inner_name, nested, grammar, depth + 1)?
            }
            _ => return None,
        };
        if space && !output.ends_with('(') && !output.ends_with(' ') {
            output.push(' ');
        }
        space = false;
        output.push_str(&piece);
    }
    output.push(')');
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn css(input: &str) -> String {
        match validate(input) {
            Ok(valid) => valid.css,
            Err(rejection) => panic!("{input:?} rejected: {rejection:?}"),
        }
    }

    #[test]
    fn plain_rules_are_rewritten_canonically() {
        assert_eq!(
            css("BODY  >  .Main #top , a:hover { Background-Color : #FFF ; color: rgb(10 20 30 / 50%) !important }"),
            "body > .Main #top, a:hover{background-color:#fff;color:rgb(10 20 30 / 50%)!important}\n"
        );
        assert_eq!(
            css("p{font-family: \"Iowan Old Style\", Georgia, serif; font-size: 1.125rem; line-height: 1.6; max-width: calc(100% - 2rem)}"),
            "p{font-family:\"Iowan Old Style\", georgia, serif;font-size:1.125rem;line-height:1.6;max-width:calc(100% - 2rem)}\n"
        );
        assert_eq!(
            css("@media (prefers-color-scheme: dark) and (min-width: 600px) { html { filter: invert(0.9) hue-rotate(180deg) } }"),
            "@media (prefers-color-scheme: dark) and (min-width: 600px){\nhtml{filter:invert(0.9) hue-rotate(180deg)}\n}\n"
        );
        assert_eq!(
            css("li:nth-child(2n+1):not(.ad, [hidden]) input[type=\"search\" i]::placeholder{opacity:.5}"),
            "li:nth-child(2n+1):not(.ad, [hidden]) input[type=\"search\" i]::placeholder{opacity:0.5}\n"
        );
    }

    #[test]
    fn disallowed_declarations_and_rules_are_dropped_not_forwarded() {
        let valid = validate(
            "a{position:fixed;z-index:9999;color:red;transform:scale(2);cursor:pointer;background:linear-gradient(red,blue)}\
             .x{position:absolute}\
             div:has(> img){color:red}\
             b{color:var(--x)}\
             i{width:12furlongs}",
        )
        .unwrap();
        assert_eq!(valid.css, "a{color:red}\n");
        assert_eq!(valid.rules, 1);
        assert!(valid.dropped >= 9);
    }

    #[test]
    fn loading_injecting_and_smuggling_reject_the_whole_sheet() {
        for (input, rejection) in [
            (
                "a{background:url(https://evil.example/x.png)}",
                Rejection::Forbidden,
            ),
            (
                "a{background:url(\"https://evil.example/x.png\")}",
                Rejection::Forbidden,
            ),
            (
                "a{background-image:image-set(\"x.png\" 1x)}",
                Rejection::Forbidden,
            ),
            (
                "a{background:-webkit-image-set(\"x.png\" 1x)}",
                Rejection::Forbidden,
            ),
            ("a::after{content:attr(data-x)}", Rejection::Forbidden),
            (
                "a::after{content:\"Your session expired, call 555\"}",
                Rejection::Forbidden,
            ),
            ("a{width:expression(alert(1))}", Rejection::Forbidden),
            ("a{-moz-binding:x}", Rejection::Forbidden),
            ("a{behavior:x}", Rejection::Forbidden),
            (
                "@import \"https://evil.example/a.css\";",
                Rejection::Forbidden,
            ),
            ("@import url(x.css); a{color:red}", Rejection::Forbidden),
            (
                "@font-face{font-family:x;src:local(x)}",
                Rejection::Forbidden,
            ),
            (
                "@namespace svg url(http://www.w3.org/2000/svg);",
                Rejection::Forbidden,
            ),
            ("@supports (color:red){a{color:red}}", Rejection::Forbidden),
            ("@keyframes x{from{color:red}}", Rejection::Forbidden),
            ("a{color:red}/* hidden */", Rejection::Forbidden),
            ("a{background:u\\72l(x)}", Rejection::Forbidden),
            ("a{color:\\72 ed}", Rejection::Forbidden),
            ("<!-- a{color:red} -->", Rejection::Forbidden),
            ("a{color:red}\u{0}", Rejection::Forbidden),
            ("a{color:red}}", Rejection::Malformed),
            ("a{color:\"unterminated\n}", Rejection::Malformed),
            ("a{b{c{d{e{f{color:red}}}}}}", Rejection::Empty),
            (
                "@media screen{@media screen{@media screen{@media screen{a{color:red}}}}}",
                Rejection::Malformed,
            ),
            ("a{position:fixed}", Rejection::Empty),
            ("", Rejection::Empty),
        ] {
            assert_eq!(validate(input), Err(rejection), "{input:?}");
        }
        // CSS closes blocks left open at the end; the output is closed either way.
        assert_eq!(css("a{color:red"), "a{color:red}\n");
        let oversized = "a{color:red}".repeat(MAX_INPUT_BYTES / 10);
        assert_eq!(validate(&oversized), Err(Rejection::TooLarge));
    }

    #[test]
    fn media_queries_are_limited_to_scheme_and_width() {
        for query in [
            "@media print{a{color:red}}",
            "@media (hover: hover){a{color:red}}",
            "@media (min-width: calc(1px)){a{color:red}}",
            "@media (min-width: 600px) or (max-width: 10px){a{color:red}}",
            "@media (prefers-color-scheme: no-preference){a{color:red}}",
        ] {
            assert_eq!(validate(query), Err(Rejection::Empty), "{query}");
        }
    }

    #[test]
    fn cards_can_be_laid_out_and_content_holds_only_separators_and_counters() {
        assert_eq!(
            css(".itemlist{display:grid;grid-template-columns:repeat(auto-fill, minmax(280px, 1fr));gap:12px}"),
            ".itemlist{display:grid;grid-template-columns:repeat(auto-fill, minmax(280px, 1fr));gap:12px}\n"
        );
        assert_eq!(
            css("tr.athing{display:flex;flex-wrap:wrap;align-items:baseline;order:2}td{display:block}"),
            "tr.athing{display:flex;flex-wrap:wrap;align-items:baseline;order:2}\ntd{display:block}\n"
        );
        assert_eq!(
            css(".subline a+a::before{content:\" · \"}li::before{content:counter(item) \". \";counter-increment:item}"),
            ".subline a + a::before{content:\" · \"}\nli::before{content:counter(item) \". \";counter-increment:item}\n"
        );
        assert_eq!(
            css("a{grid-template-columns:repeat(99, 1fr);height:0;position:fixed;color:red}"),
            "a{color:red}\n"
        );
        for spoof in [
            "a::after{content:\"Verified by Quasar\"}",
            "a::after{content:\"$0.00\"}",
            "a::after{content:counter(x) \"Call now\"}",
            "a::after{content:open-quote}",
        ] {
            assert_eq!(validate(spoof), Err(Rejection::Forbidden), "{spoof}");
        }
    }

    #[test]
    fn rule_count_is_capped() {
        let many = "a{color:red}".repeat(MAX_RULES + 20);
        let valid = validate(&many[..many.len().min(MAX_INPUT_BYTES)]).unwrap();
        assert_eq!(usize::from(valid.rules), MAX_RULES);
    }

    proptest! {
        #[test]
        fn output_is_a_fixed_point_and_never_carries_forbidden_text(input in "[a-z{}:;#.,()\\[\\]\"'@!/*>+~ 0-9%-]{0,400}") {
            if let Ok(valid) = validate(&input) {
                let again = validate(&valid.css).expect("own output is admitted");
                prop_assert_eq!(&again.css, &valid.css);
                let lower = valid.css.to_ascii_lowercase();
                for needle in ["url(", "@import", "/*", "\\", "attr(", "expression", "image"] {
                    prop_assert!(!lower.contains(needle), "{needle} in {lower}");
                }
                prop_assert!(valid.css.len() <= MAX_OUTPUT_BYTES);
            }
        }

        #[test]
        fn arbitrary_text_never_panics(input in "\\PC{0,600}") {
            let _ = validate(&input);
        }
    }
}
