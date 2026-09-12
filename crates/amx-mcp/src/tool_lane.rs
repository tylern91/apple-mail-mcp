//! Filters which tools appear in `tools/list` based on [`Config::read_only`].
//!
//! Mutating/send tools disappear from the list entirely when read-only mode is active, rather
//! than being present-but-refused — an agent should never see a tool it cannot call.

/// Which capability class a tool belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolLane {
    Read,
    Diagnostic,
    Mutate,
    Send,
}

impl ToolLane {
    fn visible_when_read_only(self) -> bool {
        matches!(self, Self::Read | Self::Diagnostic)
    }
}

/// A tool's registration metadata, independent of its concrete `rmcp` request/response types.
#[derive(Debug, Clone, Copy)]
pub struct ToolDescriptor {
    pub name: &'static str,
    pub lane: ToolLane,
    /// Mirrors the MCP `readOnlyHint` annotation — every `Read`/`Diagnostic` tool must set this.
    pub read_only_hint: bool,
}

/// Filters `catalog` down to the tools that should appear in `tools/list` given the current
/// read-only setting.
pub fn visible_tools(catalog: &[ToolDescriptor], read_only: bool) -> Vec<ToolDescriptor> {
    catalog
        .iter()
        .copied()
        .filter(|tool| !read_only || tool.lane.visible_when_read_only())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: &[ToolDescriptor] = &[
        ToolDescriptor {
            name: "search_messages",
            lane: ToolLane::Read,
            read_only_hint: true,
        },
        ToolDescriptor {
            name: "doctor",
            lane: ToolLane::Diagnostic,
            read_only_hint: true,
        },
        ToolDescriptor {
            name: "move_message",
            lane: ToolLane::Mutate,
            read_only_hint: false,
        },
        ToolDescriptor {
            name: "send_message",
            lane: ToolLane::Send,
            read_only_hint: false,
        },
    ];

    #[test]
    fn read_only_mode_hides_mutate_and_send_tools() {
        let visible = visible_tools(CATALOG, true);
        let names: Vec<_> = visible.iter().map(|t| t.name).collect();

        assert_eq!(names, ["search_messages", "doctor"]);
    }

    #[test]
    fn normal_mode_exposes_every_tool() {
        let visible = visible_tools(CATALOG, false);

        assert_eq!(visible.len(), CATALOG.len());
    }
}
