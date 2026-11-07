use markup5ever_rcdom::{Handle, NodeData};

pub fn descendant_nodes(node: &Handle) -> Vec<Handle> {
    let mut nodes = Vec::new();
    let mut pending = node
        .children
        .borrow()
        .iter()
        .rev()
        .cloned()
        .collect::<Vec<_>>();
    while let Some(node) = pending.pop() {
        pending.extend(node.children.borrow().iter().rev().cloned());
        nodes.push(node);
    }
    nodes
}

pub fn element_name(node: &Handle) -> Option<&str> {
    match &node.data {
        NodeData::Element {
            name, ..
        } => Some(name.local.as_ref()),
        _ => None,
    }
}

pub fn attribute(node: &Handle, attribute_name: &str) -> Option<String> {
    match &node.data {
        NodeData::Element {
            attrs, ..
        } => attrs
            .borrow()
            .iter()
            .find(|attribute| attribute.name.local.as_ref() == attribute_name)
            .map(|attribute| attribute.value.to_string()),
        _ => None,
    }
}
