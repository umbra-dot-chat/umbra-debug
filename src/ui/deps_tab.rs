//! Dependency Graph tab — visualize causal event chains as a tree.
//!
//! Builds a tree showing how events trigger other events:
//! dispatchers -> listeners, renders following dispatches, etc.
//! Events within 16ms of each other are grouped into frames.
//!
//! Keybindings (when Deps tab is active):
//!   Enter = expand/collapse node
//!   f     = filter by subtree (focus on selected node)
//!   t     = toggle timing display
//!   c     = toggle count display
//!   j/k   = scroll up/down
//!   r     = rebuild graph from current events

use std::collections::HashMap;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{App, TraceEvent};

/// A node in the dependency graph tree.
#[derive(Debug, Clone)]
pub struct DepsNode {
    /// Display name (function name or group label).
    pub name: String,
    /// Event category for coloring.
    pub category: String,
    /// Number of times this event path was seen.
    pub event_count: u64,
    /// Total duration in milliseconds across all occurrences.
    pub total_duration_ms: f64,
    /// Children triggered by this node.
    pub children: Vec<DepsNode>,
    /// Whether this node is collapsed in the tree view.
    pub collapsed: bool,
    /// Depth level (for indentation).
    depth: usize,
}

impl DepsNode {
    fn new(name: &str, category: &str) -> Self {
        Self {
            name: name.to_string(),
            category: category.to_string(),
            event_count: 0,
            total_duration_ms: 0.0,
            children: Vec::new(),
            collapsed: false,
            depth: 0,
        }
    }
}

/// State for the dependency graph tab.
pub struct DepsState {
    /// Root nodes of the dependency graph.
    pub roots: Vec<DepsNode>,
    /// Flattened list of visible nodes for scrolling.
    pub visible: Vec<FlatNode>,
    /// Currently selected index in the visible list.
    pub selected: usize,
    /// Whether to show timing information.
    pub show_timing: bool,
    /// Whether to show event counts.
    pub show_counts: bool,
    /// Scroll offset.
    pub scroll_offset: usize,
    /// Subtree filter: only show children of this node.
    pub subtree_filter: Option<String>,
    /// Number of events used to build the current graph.
    pub event_count_at_build: usize,
}

/// A flattened node for rendering.
#[derive(Debug, Clone)]
pub struct FlatNode {
    /// Display name.
    pub name: String,
    /// Event category for coloring.
    pub category: String,
    /// Indentation depth.
    pub depth: usize,
    /// Event count.
    pub event_count: u64,
    /// Total duration.
    pub total_duration_ms: f64,
    /// Whether this node has children.
    pub has_children: bool,
    /// Whether this node is collapsed.
    pub collapsed: bool,
    /// Tree drawing prefix (connectors like "├─", "└─", "│").
    pub prefix: String,
}

impl DepsState {
    /// Create a new empty deps state.
    pub fn new() -> Self {
        Self {
            roots: Vec::new(),
            visible: Vec::new(),
            selected: 0,
            show_timing: true,
            show_counts: true,
            scroll_offset: 0,
            subtree_filter: None,
            event_count_at_build: 0,
        }
    }

    /// Build the dependency graph from a slice of trace events.
    pub fn build_from_events(&mut self, events: &[TraceEvent]) {
        self.event_count_at_build = events.len();
        self.roots = build_dependency_tree(events);
        self.flatten();
    }

    /// Toggle collapse on the currently selected node.
    pub fn toggle_selected(&mut self) {
        if let Some(node) = self.visible.get(self.selected) {
            let name = node.name.clone();
            let depth = node.depth;
            toggle_node(&mut self.roots, &name, depth);
            self.flatten();
        }
    }

    /// Set subtree filter to the currently selected node.
    pub fn filter_subtree(&mut self) {
        if let Some(node) = self.visible.get(self.selected) {
            if self.subtree_filter.as_deref() == Some(&node.name) {
                // Toggle off
                self.subtree_filter = None;
            } else {
                self.subtree_filter = Some(node.name.clone());
            }
            self.flatten();
        }
    }

    /// Move selection up.
    pub fn select_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move selection down.
    pub fn select_down(&mut self) {
        if !self.visible.is_empty() {
            self.selected = (self.selected + 1).min(self.visible.len() - 1);
        }
    }

    /// Flatten the tree into a visible list for rendering.
    fn flatten(&mut self) {
        self.visible.clear();

        let roots = if let Some(ref filter) = self.subtree_filter {
            // Find the matching node and show its children
            match find_node(&self.roots, filter) {
                Some(node) => vec![node.clone()],
                None => {
                    self.subtree_filter = None;
                    self.roots.clone()
                }
            }
        } else {
            self.roots.clone()
        };

        for (i, root) in roots.iter().enumerate() {
            let is_last = i == roots.len() - 1;
            flatten_node(root, 0, &mut self.visible, "", is_last);
        }

        // Clamp selected to new bounds
        if !self.visible.is_empty() {
            self.selected = self.selected.min(self.visible.len() - 1);
        } else {
            self.selected = 0;
        }
    }
}

/// Flatten a tree node recursively, building tree connectors.
fn flatten_node(
    node: &DepsNode,
    depth: usize,
    out: &mut Vec<FlatNode>,
    parent_prefix: &str,
    is_last: bool,
) {
    let connector = if depth == 0 {
        String::new()
    } else if is_last {
        format!("{parent_prefix}\u{2514}\u{2500} ")
    } else {
        format!("{parent_prefix}\u{251c}\u{2500} ")
    };

    out.push(FlatNode {
        name: node.name.clone(),
        category: node.category.clone(),
        depth,
        event_count: node.event_count,
        total_duration_ms: node.total_duration_ms,
        has_children: !node.children.is_empty(),
        collapsed: node.collapsed,
        prefix: connector,
    });

    if !node.collapsed {
        let child_prefix = if depth == 0 {
            String::new()
        } else if is_last {
            format!("{parent_prefix}   ")
        } else {
            format!("{parent_prefix}\u{2502}  ")
        };

        for (i, child) in node.children.iter().enumerate() {
            let child_is_last = i == node.children.len() - 1;
            flatten_node(child, depth + 1, out, &child_prefix, child_is_last);
        }
    }
}

/// Find a node by name in the tree.
fn find_node<'a>(roots: &'a [DepsNode], name: &str) -> Option<&'a DepsNode> {
    for root in roots {
        if root.name == name {
            return Some(root);
        }
        if let Some(found) = find_node(&root.children, name) {
            return Some(found);
        }
    }
    None
}

/// Toggle collapse state of a node identified by name and depth.
fn toggle_node(roots: &mut [DepsNode], name: &str, target_depth: usize) {
    for root in roots.iter_mut() {
        if root.name == name && root.depth == target_depth {
            root.collapsed = !root.collapsed;
            return;
        }
        toggle_node(&mut root.children, name, target_depth);
    }
}

/// Frame boundary threshold: events within 16ms are in the same frame.
const FRAME_THRESHOLD_MS: f64 = 16.0;

/// Build the dependency tree from trace events.
///
/// Groups events into frames (16ms windows), then within each frame:
/// - _dispatchEvent calls become parents of subsequent listener calls
/// - Render events following dispatches are children
/// - SQL events following WASM calls are children
fn build_dependency_tree(events: &[TraceEvent]) -> Vec<DepsNode> {
    if events.is_empty() {
        return Vec::new();
    }

    // Step 1: Group events into frames (16ms windows)
    let frames = group_into_frames(events);

    // Step 2: For each frame, build a local tree
    // Step 3: Aggregate across frames into a global tree
    let mut global: HashMap<String, DepsNode> = HashMap::new();

    for frame in &frames {
        let local_roots = build_frame_tree(frame);
        for local_root in local_roots {
            merge_into_global(&mut global, local_root);
        }
    }

    // Sort by event count descending
    let mut roots: Vec<DepsNode> = global.into_values().collect();
    roots.sort_by(|a, b| b.event_count.cmp(&a.event_count));

    // Assign depths
    for root in &mut roots {
        assign_depths(root, 0);
    }

    roots
}

/// Group events into frames based on timestamp proximity.
fn group_into_frames(events: &[TraceEvent]) -> Vec<Vec<&TraceEvent>> {
    let mut frames: Vec<Vec<&TraceEvent>> = Vec::new();
    let mut current_frame: Vec<&TraceEvent> = Vec::new();
    let mut frame_start = events[0].ts;

    for ev in events {
        if ev.ts - frame_start > FRAME_THRESHOLD_MS && !current_frame.is_empty() {
            frames.push(current_frame);
            current_frame = Vec::new();
            frame_start = ev.ts;
        }
        current_frame.push(ev);
    }

    if !current_frame.is_empty() {
        frames.push(current_frame);
    }

    frames
}

/// Build a tree for a single frame of events.
fn build_frame_tree(frame: &[&TraceEvent]) -> Vec<DepsNode> {
    if frame.is_empty() {
        return Vec::new();
    }

    let mut roots: Vec<DepsNode> = Vec::new();
    let mut stack: Vec<DepsNode> = Vec::new();

    for ev in frame {
        let mut node = DepsNode::new(&ev.func, &ev.cat);
        node.event_count = 1;
        node.total_duration_ms = ev.dur_ms;

        // Determine parentage based on patterns:
        // 1. If this is a render event and previous is a dispatch, it's a child
        // 2. If this is a SQL event and previous is a WASM event, it's a child
        // 3. If this is a listener and previous is a dispatch, it's a child
        let is_child = is_likely_child(ev, &stack);

        if is_child && !stack.is_empty() {
            stack.push(node);
        } else {
            // Pop everything from the stack and nest
            flush_stack(&mut stack, &mut roots);
            stack.push(node);
        }
    }

    // Flush remaining stack
    flush_stack(&mut stack, &mut roots);

    roots
}

/// Check if an event is likely a child of the current stack top.
fn is_likely_child(ev: &TraceEvent, stack: &[DepsNode]) -> bool {
    if stack.is_empty() {
        return false;
    }

    let parent = &stack[stack.len() - 1];

    // Dispatch events trigger listeners
    if parent.name.contains("dispatch") || parent.name.contains("_dispatch") {
        return true;
    }

    // WASM calls trigger SQL operations
    if parent.category == "wasm" && ev.cat == "sql" {
        return true;
    }

    // Events with the same category in quick succession (same frame)
    // are likely part of the same call chain
    if parent.category == ev.cat
        && (ev.cat == "render" || ev.cat == "wasm")
    {
        // Render cascades
        return true;
    }

    // If a SQL context matches the parent name
    if let Some(ref ctx) = ev.sql_context {
        if parent.name.contains(ctx.as_str()) || ctx.contains(&parent.name) {
            return true;
        }
    }

    false
}

/// Flush the stack into nested nodes and push to roots.
fn flush_stack(stack: &mut Vec<DepsNode>, roots: &mut Vec<DepsNode>) {
    if stack.is_empty() {
        return;
    }

    // Build nested structure from stack (first = parent, rest = children)
    let mut nodes: Vec<DepsNode> = std::mem::take(stack);
    while nodes.len() > 1 {
        let child = nodes.pop().unwrap();
        if let Some(parent) = nodes.last_mut() {
            parent.children.push(child);
        }
    }

    if let Some(root) = nodes.pop() {
        roots.push(root);
    }
}

/// Merge a local frame root into the global tree.
fn merge_into_global(global: &mut HashMap<String, DepsNode>, local: DepsNode) {
    let entry = global.entry(local.name.clone()).or_insert_with(|| {
        let mut n = DepsNode::new(&local.name, &local.category);
        n.event_count = 0;
        n.total_duration_ms = 0.0;
        n
    });

    entry.event_count += local.event_count;
    entry.total_duration_ms += local.total_duration_ms;

    // Merge children
    for child in local.children {
        merge_child(entry, child);
    }
}

/// Merge a child node into a parent, aggregating counts.
fn merge_child(parent: &mut DepsNode, child: DepsNode) {
    if let Some(existing) = parent.children.iter_mut().find(|c| c.name == child.name) {
        existing.event_count += child.event_count;
        existing.total_duration_ms += child.total_duration_ms;
        for grandchild in child.children {
            merge_child(existing, grandchild);
        }
    } else {
        parent.children.push(child);
    }
}

/// Recursively assign depth values to nodes.
fn assign_depths(node: &mut DepsNode, depth: usize) {
    node.depth = depth;
    for child in &mut node.children {
        assign_depths(child, depth + 1);
    }
}

/// Render the Dependency Graph tab.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Dependency Graph ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::LightGreen));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let deps = match &app.deps_state {
        Some(d) => d,
        None => {
            render_no_graph(frame, inner);
            return;
        }
    };

    if deps.visible.is_empty() {
        render_no_graph(frame, inner);
        return;
    }

    // Reserve 1 line for the status bar
    if inner.height < 2 {
        return;
    }
    let content_area = Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    };
    let status_area = Rect {
        y: inner.y + content_area.height,
        height: 1,
        ..inner
    };

    render_tree(frame, deps, content_area);
    render_deps_status_bar(frame, deps, status_area);
}

/// Render the "no graph" help screen.
fn render_no_graph(frame: &mut Frame, area: Rect) {
    let help = vec![
        Line::from(""),
        Line::from(Span::styled(
            "No dependency graph built yet.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press 'r' to build from current events, or wait for events.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Keybindings:",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  Enter = expand/collapse    f = filter subtree",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            "  t = toggle timing    c = toggle counts    r = rebuild",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            "  j/k = scroll up/down",
            Style::default().fg(Color::Yellow),
        )),
    ];
    let paragraph = Paragraph::new(help).alignment(Alignment::Center);
    frame.render_widget(paragraph, area);
}

/// Render the tree view.
fn render_tree(frame: &mut Frame, deps: &DepsState, area: Rect) {
    let visible_height = area.height as usize;
    if visible_height == 0 {
        return;
    }

    let total = deps.visible.len();

    // Calculate scroll to keep selected visible
    let scroll = if deps.selected >= deps.scroll_offset + visible_height {
        deps.selected - visible_height + 1
    } else if deps.selected < deps.scroll_offset {
        deps.selected
    } else {
        deps.scroll_offset
    };

    let end = total.min(scroll + visible_height);
    let visible_nodes = &deps.visible[scroll..end];

    let items: Vec<ListItem> = visible_nodes
        .iter()
        .enumerate()
        .map(|(win_idx, node)| {
            let global_idx = scroll + win_idx;
            let is_selected = global_idx == deps.selected;

            let cat_color = category_color(&node.category);
            let collapse_indicator = if node.has_children {
                if node.collapsed { "[+] " } else { "[-] " }
            } else {
                "    "
            };

            let mut spans = vec![
                Span::styled(
                    &node.prefix,
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    collapse_indicator,
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    &node.name,
                    Style::default().fg(cat_color),
                ),
            ];

            if deps.show_counts && node.event_count > 0 {
                spans.push(Span::styled(
                    format!(" x{}", node.event_count),
                    Style::default().fg(Color::DarkGray),
                ));
            }

            if deps.show_timing && node.total_duration_ms > 0.0 {
                let dur_color = if node.total_duration_ms > 1000.0 {
                    Color::Red
                } else if node.total_duration_ms > 100.0 {
                    Color::Yellow
                } else {
                    Color::DarkGray
                };
                spans.push(Span::styled(
                    format!(" ({:.1}ms)", node.total_duration_ms),
                    Style::default().fg(dur_color),
                ));
            }

            let line = Line::from(spans);

            if is_selected {
                ListItem::new(line).style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::LightGreen),
                )
            } else {
                ListItem::new(line)
            }
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

/// Render the deps-specific status bar.
fn render_deps_status_bar(frame: &mut Frame, deps: &DepsState, area: Rect) {
    let mut parts = vec![
        format!("{} nodes", deps.visible.len()),
        format!("{} roots", deps.roots.len()),
        format!("from {} events", deps.event_count_at_build),
    ];

    if let Some(ref filter) = deps.subtree_filter {
        parts.push(format!("subtree: {filter}"));
    }

    if !deps.show_timing {
        parts.push("timing: off".to_string());
    }
    if !deps.show_counts {
        parts.push("counts: off".to_string());
    }

    let status = parts.join(" | ");
    let bar = Paragraph::new(status).style(
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray),
    );
    frame.render_widget(bar, area);
}

/// Get the display color for an event category.
fn category_color(cat: &str) -> Color {
    match cat {
        "wasm" => Color::Cyan,
        "sql" => Color::Yellow,
        "net" | "network" => Color::Green,
        "mem" => Color::Magenta,
        "err" | "error" => Color::Red,
        "render" => Color::LightBlue,
        "service" => Color::LightCyan,
        _ => Color::White,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TraceEvent;

    fn make_event(seq: u64, func: &str, cat: &str, ts: f64) -> TraceEvent {
        TraceEvent {
            seq,
            ts,
            cat: cat.to_string(),
            func: func.to_string(),
            arg_bytes: 0,
            arg_preview: None,
            dur_ms: 1.0,
            mem_before: 0,
            mem_after: 0,
            mem_growth: 0,
            sql_context: None,
            client_id: "test".to_string(),
            err: None,
            meta: None,
        }
    }

    #[test]
    fn test_build_empty() {
        let events: Vec<TraceEvent> = Vec::new();
        let tree = build_dependency_tree(&events);
        assert!(tree.is_empty());
    }

    #[test]
    fn test_build_single_event() {
        let events = vec![make_event(0, "init", "wasm", 0.0)];
        let tree = build_dependency_tree(&events);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "init");
        assert_eq!(tree[0].event_count, 1);
    }

    #[test]
    fn test_frame_grouping() {
        let events = vec![
            make_event(0, "a", "wasm", 0.0),
            make_event(1, "b", "wasm", 5.0),   // Same frame (<16ms)
            make_event(2, "c", "wasm", 100.0),  // New frame
        ];
        let frames = group_into_frames(&events);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].len(), 2);
        assert_eq!(frames[1].len(), 1);
    }

    #[test]
    fn test_dispatch_creates_parent() {
        let events = vec![
            make_event(0, "_dispatchEvent", "wasm", 0.0),
            make_event(1, "handleMessage", "wasm", 1.0),
            make_event(2, "renderChat", "render", 2.0),
        ];
        let tree = build_dependency_tree(&events);
        // _dispatchEvent should be a root with children
        let dispatch = tree.iter().find(|n| n.name == "_dispatchEvent");
        assert!(dispatch.is_some());
        let dispatch = dispatch.unwrap();
        assert!(!dispatch.children.is_empty());
    }

    #[test]
    fn test_deps_state_toggle() {
        let events = vec![
            make_event(0, "_dispatchEvent", "wasm", 0.0),
            make_event(1, "handler", "wasm", 1.0),
        ];
        let mut state = DepsState::new();
        state.build_from_events(&events);

        let initial_visible = state.visible.len();
        assert!(initial_visible > 0);

        // Toggle collapse on the first node
        state.selected = 0;
        state.toggle_selected();

        // After collapsing, should have fewer visible nodes
        assert!(state.visible.len() <= initial_visible);
    }

    #[test]
    fn test_flatten_tree_connectors() {
        let mut root = DepsNode::new("root", "wasm");
        root.event_count = 1;

        let mut child1 = DepsNode::new("child1", "wasm");
        child1.event_count = 1;

        let mut child2 = DepsNode::new("child2", "sql");
        child2.event_count = 1;

        root.children.push(child1);
        root.children.push(child2);

        let mut visible = Vec::new();
        flatten_node(&root, 0, &mut visible, "", true);

        // Should have root + 2 children = 3 nodes
        assert_eq!(visible.len(), 3);
        assert_eq!(visible[0].name, "root");
        assert_eq!(visible[1].name, "child1");
        assert_eq!(visible[2].name, "child2");
        // Children should have non-empty prefixes
        assert!(!visible[1].prefix.is_empty());
        assert!(!visible[2].prefix.is_empty());
    }

    #[test]
    fn test_merge_aggregates_counts() {
        let mut global: HashMap<String, DepsNode> = HashMap::new();

        let mut node1 = DepsNode::new("func_a", "wasm");
        node1.event_count = 3;
        node1.total_duration_ms = 10.0;

        let mut node2 = DepsNode::new("func_a", "wasm");
        node2.event_count = 5;
        node2.total_duration_ms = 20.0;

        merge_into_global(&mut global, node1);
        merge_into_global(&mut global, node2);

        let merged = global.get("func_a").unwrap();
        assert_eq!(merged.event_count, 8);
        assert!((merged.total_duration_ms - 30.0).abs() < 0.001);
    }
}
