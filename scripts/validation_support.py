#!/usr/bin/env python3
"""Shared source-validation helpers for HomeServer CI contracts."""
from __future__ import annotations

import re


def extract_balanced_call_argument(source: str, call: str) -> str | None:
    """Return the argument text for the first balanced function call."""
    marker = f"{call}("
    start = source.find(marker)
    if start < 0:
        return None

    open_index = start + len(marker) - 1
    depth = 0
    in_string = False
    escaped = False
    index = open_index

    while index < len(source):
        char = source[index]
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            index += 1
            continue

        if char == '"':
            in_string = True
        elif char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                return source[open_index + 1 : index]
        index += 1

    return None


def secured_router_body(source: str) -> str | None:
    """Return the router expression wrapped by http::secure(...)."""
    return extract_balanced_call_argument(source, "http::secure")


def router_component_is_secured(source: str, component: str) -> bool:
    """Accept state or state.clone() only when merged inside http::secure."""
    body = secured_router_body(source)
    if body is None:
        return False

    pattern = re.compile(
        rf"\.merge\(\s*{re.escape(component)}::router\(\s*"
        rf"state(?:\.clone\(\))?\s*\)\s*\)"
    )
    return pattern.search(body) is not None


def base_router_is_secured(source: str) -> bool:
    """Verify the root HTTP router is part of the secured router expression."""
    body = secured_router_body(source)
    if body is None:
        return False
    return (
        re.search(r"http::router\(\s*state(?:\.clone\(\))?\s*\)", body)
        is not None
    )


def merged_value_is_secured(source: str, value: str) -> bool:
    """Verify a preconstructed Router value is merged inside http::secure."""
    body = secured_router_body(source)
    if body is None:
        return False
    return re.search(rf"\.merge\(\s*{re.escape(value)}\s*\)", body) is not None
