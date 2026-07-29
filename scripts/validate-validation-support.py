#!/usr/bin/env python3
"""Regression tests for source-level HomeServer validation helpers."""
from __future__ import annotations

from validation_support import (
    base_router_is_secured,
    merged_value_is_secured,
    router_component_is_secured,
)

SAFE_STATE = """
let router = http::secure(
    http::router(state)
        .merge(mcp_runtime::router(state))
        .merge(registry_router),
);
"""

SAFE_CLONE = """
let router = http::secure(
    http::router(state.clone())
        .merge(mcp_runtime::router(state.clone()))
        .merge(registry_router),
);
"""

UNSAFE_OUTSIDE = """
let insecure = mcp_runtime::router(state.clone());
let router = http::secure(http::router(state.clone()));
"""

for source in (SAFE_STATE, SAFE_CLONE):
    assert base_router_is_secured(source)
    assert router_component_is_secured(source, "mcp_runtime")
    assert merged_value_is_secured(source, "registry_router")

assert base_router_is_secured(UNSAFE_OUTSIDE)
assert not router_component_is_secured(UNSAFE_OUTSIDE, "mcp_runtime")
assert not merged_value_is_secured(UNSAFE_OUTSIDE, "registry_router")

print("Semantic secured-router validation helper tests passed.")
