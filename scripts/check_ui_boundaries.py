#!/usr/bin/env python3
"""Enforce the desktop UI ownership seams without Cargo or third-party modules.

This is a lexical architecture check, not a Rust type checker. It reads every UI
Rust file (including tests and dormant modules), strips comments and literals,
expands grouped imports, resolves local import aliases and ``super`` paths, and
checks public mutable-return signatures. Cargo still verifies Rust visibility.

Run ``python scripts/check_ui_boundaries.py`` from any directory. Its regression
tests are ``python -m unittest discover -s scripts/tests -p test_check_ui_boundaries.py``.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import re
import sys
from typing import Iterable


UI_PATH = Path("crates/miaominal-ui/src/ui")
OWNERS = {"application", "components", "contracts", "features", "workspace", "shell"}
OBSOLETE_OWNER_SOURCES = (
    "shell/controllers/session.rs",
    "shell/controllers/agent.rs",
    "shell/controllers/sftp.rs",
    "shell/controllers/settings.rs",
    "shell/controllers/keychain.rs",
    "shell/workspace.rs",
)
MUTABLE_GUARDS = {"RefMut", "MutexGuard", "RwLockWriteGuard", "MappedMutexGuard"}
MUTABLE_CONTAINERS = {"RefCell", "Cell", "Mutex", "RwLock"}
# Session implements pane input policy while Workspace retains the pane identity.
# These two adapter methods borrow only transient Session-defined input state.
# They expose neither runtime resources nor controller/catalog/tab internals.
MUTABLE_ADAPTER_METHODS = {
    (("crate", "ui", "features", "session", "input", "gestures"),
     "TerminalInputHost", "active_terminal_pane_mut", "PaneViewState"),
    (("crate", "ui", "features", "session", "input", "gestures"),
     "TerminalInputHost", "terminal_drag_mut", "TerminalDragState"),
}
RAW_STRING = re.compile(r'(?:br|cr|r)(#*)"')
CHARACTER = re.compile(r"'(?:\\(?:u\{[^}]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\n])'")
LIFETIME = re.compile(r"'[A-Za-z_][A-Za-z_0-9]*")
IDENTIFIER = re.compile(r"(?:r#)?[A-Za-z_][A-Za-z_0-9]*")
PATH_COMPONENT = re.compile(r"[A-Za-z_][A-Za-z_0-9]*|\*")


@dataclass(frozen=True)
class Token:
    value: str
    line: int
    kind: str = "code"


@dataclass(frozen=True, order=True)
class Failure:
    path: str
    line: int
    rule: str
    message: str

    def __str__(self) -> str:
        return f"{self.path}:{self.line}: [{self.rule}] {self.message}"


@dataclass(frozen=True)
class Import:
    path: tuple[str, ...]
    alias: str | None
    line: int
    public: bool


def lex(source: str) -> list[Token]:
    """Keep line numbers and string attributes, without treating text as code."""
    tokens: list[Token] = []
    pos, line, length = 0, 1, len(source)
    while pos < length:
        char = source[pos]
        if char.isspace():
            line += char == "\n"
            pos += 1
            continue
        if source.startswith("//", pos):
            end = source.find("\n", pos)
            pos = length if end < 0 else end
            continue
        if source.startswith("/*", pos):
            depth, end = 1, pos + 2
            while end < length and depth:
                if source.startswith("/*", end):
                    depth += 1
                    end += 2
                elif source.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    line += source[end] == "\n"
                    end += 1
            pos = end
            continue
        # Rust raw strings may contain quotes, comment markers and fake imports.
        raw = RAW_STRING.match(source, pos)
        if raw:
            start = raw.end()
            delimiter = '"' + raw.group(1)
            end = source.find(delimiter, start)
            end = length if end < 0 else end
            tokens.append(Token(source[start:end], line, "string"))
            finish = min(length, end + len(delimiter))
            line += source[pos:finish].count("\n")
            pos = finish
            continue
        if char == '"' or (char in "bc" and source[pos:pos + 2].endswith('"')):
            start = pos + (1 if char == '"' else 2)
            end = start
            while end < length:
                if source[end] == "\\":
                    end += 2
                elif source[end] == '"':
                    break
                else:
                    end += 1
            tokens.append(Token(source[start:end], line, "string"))
            finish = min(length, end + 1)
            line += source[pos:finish].count("\n")
            pos = finish
            continue
        # A lifetime is code; a quoted character is a literal.
        if char == "'":
            literal = CHARACTER.match(source, pos)
            if literal:
                pos = literal.end()
                continue
            lifetime = LIFETIME.match(source, pos)
            if lifetime:
                tokens.append(Token(lifetime.group(), line))
                pos = lifetime.end()
                continue
        identifier = IDENTIFIER.match(source, pos)
        if identifier:
            tokens.append(Token(identifier.group().removeprefix("r#"), line))
            pos = identifier.end()
            continue
        pair = source[pos:pos + 2]
        if pair in {"::", "->", "=>"}:
            tokens.append(Token(pair, line))
            pos += 2
        else:
            tokens.append(Token(char, line))
            pos += 1
    return tokens


def matching(tokens: list[Token], start: int) -> int:
    pairs = {"(": ")", "[": "]", "{": "}", "<": ">"}
    closing = pairs[tokens[start].value]
    depth = 1
    for index in range(start + 1, len(tokens)):
        value = tokens[index].value
        if value == tokens[start].value:
            depth += 1
        elif value == closing:
            depth -= 1
            if depth == 0:
                return index
    return len(tokens) - 1


def nominal_module(path: Path) -> tuple[str, ...]:
    parts = list(path.with_suffix("").parts)
    if parts[-1] == "mod":
        parts.pop()
    return ("crate", "ui", *parts)


def owner(module: tuple[str, ...]) -> tuple[str, ...]:
    if module[:2] != ("crate", "ui") or len(module) < 3:
        return ()
    return module[:4] if module[2] == "features" and len(module) >= 4 else module[:3]


def resolve(path: tuple[str, ...], module: tuple[str, ...], aliases: dict[str, tuple[str, ...]]) -> tuple[str, ...]:
    if not path:
        return ()
    visited: set[str] = set()
    while path and path[0] in aliases and path[0] not in visited:
        visited.add(path[0])
        path = aliases[path[0]] + path[1:]
    if path[0] == "crate":
        return path
    if path[0] == "self":
        return module + path[1:]
    if path[0] == "super":
        count = 0
        while count < len(path) and path[count] == "super":
            count += 1
        return module[:-count] + path[count:]
    return path


def visibility(tokens: list[Token], index: int) -> tuple[list[str], int]:
    if tokens[index].value != "pub":
        return [], index
    if index + 1 < len(tokens) and tokens[index + 1].value == "(":
        end = matching(tokens, index + 1)
        return [t.value for t in tokens[index + 2:end]], end + 1
    return ["public"], index + 1


def crosses_owner(visibility_tokens: list[str], module: tuple[str, ...]) -> bool:
    if not visibility_tokens:
        return False
    if visibility_tokens == ["self"]:
        return False
    if visibility_tokens == ["super"]:
        return owner(module[:-1]) != owner(module)
    if visibility_tokens[0] == "in":
        scope = tuple(value for value in visibility_tokens[1:] if value != "::")
        return owner(resolve(scope, module, {})) != owner(module)
    return True


def expand_imports(tokens: list[Token], public: bool) -> list[Import]:
    """Expand nested Rust use trees, including self and renamed modules."""
    result: list[Import] = []

    def branch(start: int, prefix: tuple[str, ...]) -> int:
        current = prefix
        line = tokens[start].line if start < len(tokens) else 1
        index = start
        while index < len(tokens):
            value = tokens[index].value
            if value == "{":
                index += 1
                while index < len(tokens) and tokens[index].value != "}":
                    if tokens[index].value == ",":
                        index += 1
                    else:
                        index = branch(index, current)
                return index + 1
            if value in {",", "}"}:
                break
            if value == "as":
                alias = tokens[index + 1].value
                result.append(Import(current, alias, line, public))
                return index + 2
            if value != "::":
                # The self leaf imports its prefix, rather than a child named self.
                if value != "self" or not current:
                    current += (value,)
            index += 1
        result.append(Import(current, None, line, public))
        return index

    if tokens:
        index = 0
        while index < len(tokens):
            if tokens[index].value == ",":
                index += 1
            else:
                index = branch(index, ())
    return result


def local_contexts(tokens: list[Token], module: tuple[str, ...]) -> list[tuple[str, ...]]:
    """Account for inline test modules when resolving relative paths."""
    contexts = [module] * len(tokens)
    pending: dict[int, str] = {}
    for index in range(len(tokens) - 2):
        if tokens[index].value == "mod" and tokens[index + 2].value == "{":
            pending[index + 2] = tokens[index + 1].value
    stack: list[tuple[str, ...]] = []
    current = module
    for index, token in enumerate(tokens):
        contexts[index] = current
        if token.value == "{":
            stack.append(current)
            if index in pending:
                current += (pending[index],)
        elif token.value == "}" and stack:
            current = stack.pop()
    return contexts


def module_sources(ui: Path) -> dict[Path, tuple[str, ...]]:
    """Map #[path] sources to their declared module, not their folder spelling.

    For example settings/dialogs.rs is compiled as settings::controller::dialogs.
    Using its physical path to resolve super would silently check the wrong owner.
    Unreferenced sources are scanned separately using their nominal module path.
    """
    entry = ui.with_suffix(".rs")
    if not entry.is_file():
        return {}
    modules: dict[Path, tuple[str, ...]] = {}
    pending = [(entry.resolve(), ("crate", "ui"))]
    while pending:
        path, module = pending.pop()
        if path in modules:
            continue
        modules[path] = module
        tokens = lex(path.read_text(encoding="utf-8"))
        contexts = local_contexts(tokens, module)
        attributes: dict[tuple[str, ...], str] = {}
        default_base = path.parent if path.name == "mod.rs" else path.with_suffix("")
        for index, token in enumerate(tokens):
            if token.kind == "string" and index >= 2 and [t.value for t in tokens[index - 2:index]] == ["path", "="]:
                attributes[contexts[index]] = token.value
            if token.value != "mod" or index + 2 >= len(tokens):
                continue
            name, ending = tokens[index + 1].value, tokens[index + 2].value
            context = contexts[index]
            source_attribute = attributes.pop(context, None)
            if ending != ";":
                continue
            inline = context[len(module):]
            if source_attribute is not None:
                candidates = [path.parent.joinpath(*inline, source_attribute)]
            else:
                base = default_base.joinpath(*inline)
                candidates = [base / f"{name}.rs", base / name / "mod.rs"]
            for candidate in candidates:
                if candidate.is_file() and candidate.resolve().is_relative_to(ui.resolve()):
                    pending.append((candidate.resolve(), (*context, name)))
                    break
    return modules


def check_source(path: str, source: str, module: tuple[str, ...], declared_modules: set[tuple[str, ...]] | None = None) -> list[Failure]:
    tokens = lex(source)
    contexts = local_contexts(tokens, module)
    failures: set[Failure] = set()
    aliases: dict[tuple[str, ...], dict[str, tuple[str, ...]]] = {}
    uses: list[tuple[Import, tuple[str, ...]]] = []
    imported_indexes: set[int] = set()
    public_items: dict[int, int] = {}
    public_traits: list[tuple[int, int, str]] = []
    for index, token in enumerate(tokens):
        if token.value != "pub":
            continue
        visible, declaration = visibility(tokens, index)
        if not crosses_owner(visible, contexts[index]) or declaration >= len(tokens):
            continue
        public_items[declaration] = index
        if tokens[declaration].value == "trait":
            opening = declaration + 2
            while opening < len(tokens) and tokens[opening].value not in {"{", ";"}:
                opening += 1
            if opening < len(tokens) and tokens[opening].value == "{":
                public_traits.append((opening, matching(tokens, opening), tokens[declaration + 1].value))

    def fail(line: int, rule: str, message: str) -> None:
        failures.add(Failure(path, line, rule, message))

    def check_path(target: tuple[str, ...], line: int, context: tuple[str, ...]) -> None:
        source_owner, target_owner = owner(context), owner(target)
        if not source_owner or not target_owner or source_owner == target_owner:
            return
        source_layer, target_layer = source_owner[2], target_owner[2]
        if source_layer != "shell" and target_layer == "shell":
            fail(line, "SHELL_DEPENDENCY", "owner depends on window Shell; use a contract or component")
        elif source_layer == "contracts" and target_layer in {"features", "workspace", "application"}:
            fail(line, "CONTRACT_DIRECTION", "contracts must define their seam instead of aliasing its implementation")
        elif source_layer == "components" and target_layer in {"features", "workspace", "application"}:
            fail(line, "COMPONENT_DIRECTION", "shared presentation depends on an application owner")
        elif source_layer == "features" and target_layer == "features":
            fail(line, "SIBLING_FEATURE", "features communicate through contracts, not sibling controllers or views")
        elif source_layer == "features" and target_layer in {"workspace", "application"}:
            fail(line, "FEATURE_DIRECTION", "feature needs a narrow injected port or semantic command")
        elif target_layer == "features" and ((len(target) > 5 and target[4][:1].islower()) or (declared_modules and len(target) > 4 and target in declared_modules)):
            fail(line, "FEATURE_INTERNAL", "compose the explicit feature root seam instead of reaching into its modules")
        elif target_layer == "features" and target[-1] == "*":
            fail(line, "BROAD_FEATURE_IMPORT", "feature imports must name the explicit public seam")

    # Collect use aliases first so qualified expressions cannot hide an owner path.
    for index, token in enumerate(tokens):
        if token.value != "use":
            continue
        end = index + 1
        while end < len(tokens) and tokens[end].value != ";":
            end += 1
        imported_indexes.update(range(index, end))
        context = contexts[index]
        public = index in public_items
        for item in expand_imports(tokens[index + 1:end], public):
            uses.append((item, context))

    def context_aliases(context: tuple[str, ...]) -> dict[str, tuple[str, ...]]:
        merged: dict[str, tuple[str, ...]] = {}
        for depth in range(1, len(context) + 1):
            merged.update(aliases.get(context[:depth], {}))
        return merged

    # Rust imports are order-independent. Resolve forward aliases to a fixed
    # point; a finite bound also makes malformed/cyclic imports terminate.
    for _ in range(len(uses) + 1):
        changed = False
        for item, context in uses:
            target = resolve(item.path, context, context_aliases(context))
            name = item.alias or (item.path[-1] if item.path else "_")
            if name == "self":
                name = target[-1] if target else "_"
            if name not in {"_", "*"}:
                scope_aliases = aliases.setdefault(context, {})
                if scope_aliases.get(name) != target:
                    scope_aliases[name] = target
                    changed = True
        if not changed:
            break
    for item, context in uses:
        target = resolve(item.path, context, context_aliases(context))
        check_path(target, item.line, context)
        if item.public and item.path[-1:] == ("*",) and owner(context)[2:3] == ("features",):
            fail(item.line, "BROAD_FEATURE_EXPORT", "feature public seam must use explicit re-exports")
        if item.public and declared_modules and target in declared_modules and owner(context)[2:3] == ("features",):
            fail(item.line, "PUBLIC_FEATURE_MODULE", "feature public seam must export items, not implementation modules")

    for index, token in enumerate(tokens):
        if token.kind != "code":
            continue
        context = contexts[index]
        layer = owner(context)[2:3]
        if token.value == "AppView" and layer and layer != ("shell",):
            fail(token.line, "APP_VIEW_DEPENDENCY", "owner must not retain or depend on AppView")
        if token.value == "DialogOverlaySnapshot" and layer == ("shell",):
            fail(token.line, "BUSINESS_DIALOG_DISPATCH", "Shell composes opaque owner snapshots instead of business dialog variants")
        if index not in imported_indexes and index + 1 < len(tokens) and tokens[index + 1].value == "::":
            if index > 0 and tokens[index - 1].value == "::":
                continue
            values = [token.value]
            end = index + 1
            while end + 1 < len(tokens) and tokens[end].value == "::" and PATH_COMPONENT.fullmatch(tokens[end + 1].value):
                values.append(tokens[end + 1].value)
                end += 2
            check_path(resolve(tuple(values), context, context_aliases(context)), token.line, context)

        # Public feature submodules make the implementation accessible outside
        # its owner. Explicit re-exports keep the facade reviewable.
        if layer not in {("features",), ("contracts",)}:
            continue
        trait_name = next((name for start, end, name in public_traits if start < index < end), None)
        if token.value == "pub":
            visible, declaration = visibility(tokens, index)
            if not crosses_owner(visible, context):
                continue
        elif token.value == "fn" and trait_name:
            declaration = index
        else:
            continue
        if declaration >= len(tokens):
            continue
        item = tokens[declaration].value
        if item == "mod" and layer == ("features",) and len(owner(context)) == 4:
            fail(token.line, "PUBLIC_FEATURE_MODULE", "feature implementation modules must stay owner-private")
        while item in {"async", "unsafe", "const", "extern"} and declaration + 1 < len(tokens):
            declaration += 1
            item = tokens[declaration].value
        if item != "fn":
            # Rc<RefCell<State>> fields and type aliases also allow a caller to
            # mutate canonical state, despite the absence of a _mut accessor.
            if item == "type" or (declaration + 1 < len(tokens) and tokens[declaration + 1].value == ":"):
                end = declaration + 1
                values: list[str] = []
                depth = 0
                while end < len(tokens):
                    value = tokens[end].value
                    if value in {";", "{", "}"} or (value == "," and depth == 0):
                        break
                    depth += value in {"<", "("}
                    depth -= value in {">", ")"}
                    values.append(value)
                    end += 1
                if MUTABLE_CONTAINERS.intersection(values):
                    fail(token.line, "MUTABLE_ESCAPE", "public field or alias exposes mutable owner state")
            continue
        end = declaration + 2
        while end < len(tokens) and tokens[end].value != "(":
            end += 1
        if end >= len(tokens):
            continue
        end = matching(tokens, end) + 1
        if end >= len(tokens) or tokens[end].value != "->":
            continue
        end += 1
        result: list[str] = []
        while end < len(tokens) and tokens[end].value not in {"{", ";", "where"}:
            result.append(tokens[end].value)
            end += 1
        if any(value in {"Fn", "FnMut", "FnOnce"} for value in result):
            continue
        mutable_reference = any(
            value == "&" and any(part == "mut" for part in result[j + 1:j + 3])
            for j, value in enumerate(result)
        )
        function_name = tokens[declaration + 1].value
        if any((context, trait_name, function_name, value) in MUTABLE_ADAPTER_METHODS for value in result):
            continue
        if mutable_reference or (MUTABLE_GUARDS | MUTABLE_CONTAINERS).intersection(result):
            fail(token.line, "MUTABLE_ESCAPE", "public seam returns mutable owner state; expose a command or snapshot")

    # Module-source attributes and include! cannot bypass the directory boundary.
    for index, token in enumerate(tokens):
        if token.kind != "string":
            continue
        previous = [t.value for t in tokens[max(0, index - 4):index]]
        if previous[-2:] == ["path", "="] or "include" in previous:
            destination = (Path(path).parent / token.value.replace("\\", "/")).as_posix()
            normalized = Path(destination).resolve()
            parts = normalized.parts
            # A source attribute may only compile code inside the same owner.
            target_owner = None
            for cursor in range(len(parts) - 1):
                if parts[cursor] == "ui":
                    target_owner = owner(("crate", *parts[cursor:]))
            if target_owner is None or target_owner != owner(module):
                fail(token.line, "SOURCE_REDIRECT", "module source is redirected outside its owner")
    return sorted(failures)


def check_tree(root: Path) -> list[Failure]:
    ui = root / UI_PATH
    failures: list[Failure] = []
    if not ui.is_dir():
        return [Failure(UI_PATH.as_posix(), 1, "MISSING_UI", "UI source directory is missing")]
    declared_modules = module_sources(ui)
    for path in sorted(ui.rglob("*.rs")):
        relative = path.relative_to(ui)
        module = declared_modules.get(path.resolve(), nominal_module(relative))
        if owner(module)[2:3] not in {(name,) for name in OWNERS}:
            continue
        failures.extend(check_source(path.relative_to(root).as_posix(), path.read_text(encoding="utf-8"), module, set(declared_modules.values())))
    for relative in OBSOLETE_OWNER_SOURCES:
        if (ui / relative).exists():
            failures.append(Failure((UI_PATH / relative).as_posix(), 1, "OBSOLETE_OWNER_SOURCE", "obsolete Shell state owner remains; finish its migration"))
    return sorted(set(failures))


def main(argv: Iterable[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1], help="repository root")
    arguments = parser.parse_args(argv)
    failures = check_tree(arguments.root.resolve())
    if failures:
        print("\n".join(map(str, failures)), file=sys.stderr)
        print(f"UI ownership check failed: {len(failures)} violation(s)", file=sys.stderr)
        return 1
    print("UI owner boundaries passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
