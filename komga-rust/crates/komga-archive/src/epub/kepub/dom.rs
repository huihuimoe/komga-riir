//! The document tree kepubify's `x/net/html` fork builds: an HTML5 parse with its lenient
//! self-closing extension, in an arena the transforms can edit.

use std::borrow::Cow;
use std::cell::RefCell;

use html5ever::interface::{ElemName, ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use html5ever::tree_builder::{TreeBuilder, TreeBuilderOpts};
use html5ever::{Attribute, LocalName, Namespace, QualName, TokenizerResult, local_name};

pub(super) type NodeId = usize;

pub(super) const DOCUMENT: NodeId = 0;

#[derive(Debug)]
pub(super) enum NodeData {
    Document,
    Doctype {
        name: String,
        public_id: String,
        system_id: String,
    },
    Element {
        name: QualName,
        attrs: Vec<Attribute>,
    },
    Text(String),
    Comment(String),
}

#[derive(Debug)]
pub(super) struct Node {
    pub data: NodeData,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
}

/// An arena of nodes; removed nodes stay allocated but detached.
#[derive(Debug)]
pub(super) struct Dom {
    pub nodes: Vec<Node>,
}

impl Dom {
    pub fn new_node(&mut self, data: NodeData) -> NodeId {
        self.nodes.push(Node {
            data,
            parent: None,
            children: Vec::new(),
        });
        self.nodes.len() - 1
    }

    pub fn element(&self, id: NodeId) -> Option<(&QualName, &[Attribute])> {
        match &self.nodes[id].data {
            NodeData::Element { name, attrs } => Some((name, attrs)),
            _ => None,
        }
    }

    pub fn local_name(&self, id: NodeId) -> Option<&LocalName> {
        self.element(id).map(|(name, _)| &name.local)
    }

    pub fn attr(&self, id: NodeId, key: &str) -> Option<&str> {
        let (_, attrs) = self.element(id)?;
        let attr = attrs.iter().find(|attr| &*attr.name.local == key)?;
        Some(&attr.value)
    }

    pub fn append(&mut self, parent: NodeId, child: NodeId) {
        self.detach(child);
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
    }

    pub fn insert_before(&mut self, sibling: NodeId, child: NodeId) {
        self.detach(child);
        let parent = self.nodes[sibling]
            .parent
            .expect("Insertion next to an attached node");
        let index = self.index_in_parent(parent, sibling);
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.insert(index, child);
    }

    pub fn detach(&mut self, id: NodeId) {
        if let Some(parent) = self.nodes[id].parent.take() {
            let index = self.index_in_parent(parent, id);
            self.nodes[parent].children.remove(index);
        }
    }

    pub fn reparent_children(&mut self, from: NodeId, to: NodeId) {
        for child in std::mem::take(&mut self.nodes[from].children) {
            self.nodes[child].parent = None;
            self.append(to, child);
        }
    }

    fn index_in_parent(&self, parent: NodeId, child: NodeId) -> usize {
        self.nodes[parent]
            .children
            .iter()
            .position(|&id| id == child)
            .expect("A child is among its parent's children")
    }

    /// Go `findAtom`: the first descendant element named `name`, in document order.
    pub fn find(&self, from: NodeId, name: &LocalName) -> Option<NodeId> {
        self.nodes[from].children.iter().find_map(|&child| {
            if self
                .local_name(child)
                .is_some_and(|local| local.eq_ignore_ascii_case(name))
            {
                Some(child)
            } else {
                self.find(child, name)
            }
        })
    }

    /// Appends text, merged into a preceding text node as the parser does.
    fn append_text(&mut self, parent: NodeId, text: &str) {
        if let Some(&last) = self.nodes[parent].children.last()
            && let NodeData::Text(existing) = &mut self.nodes[last].data
        {
            existing.push_str(text);
            return;
        }
        let node = self.new_node(NodeData::Text(text.to_owned()));
        self.append(parent, node);
    }
}

/// Parses `content` as kepubify does: an HTML5 document with scripting enabled, where `title`,
/// `script`, `a`, `p`, `div`, and `span` may also close themselves (`ParseOptionLenientSelfClosing`).
pub(super) fn parse(content: &str) -> Dom {
    let sink = Sink(RefCell::new(Dom {
        nodes: vec![Node {
            data: NodeData::Document,
            parent: None,
            children: Vec::new(),
        }],
    }));
    let builder = TreeBuilder::new(sink, TreeBuilderOpts::default());
    let tokenizer = Tokenizer::new(LenientSelfClosing(builder), TokenizerOpts::default());
    let input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(content));
    // Scripts and encoding declarations pause the tokenizer; neither changes the parse.
    while !matches!(tokenizer.feed(&input), TokenizerResult::Done) {}
    tokenizer.end();
    tokenizer.sink.0.sink.0.into_inner()
}

/// Ends the self-closing elements kepubify's parser lets close themselves right after they
/// start. `title` and `script` would otherwise switch the tokenizer to raw text.
struct LenientSelfClosing(TreeBuilder<NodeId, Sink>);

impl TokenSink for LenientSelfClosing {
    type Handle = NodeId;

    fn process_token(&self, token: Token, line_number: u64) -> TokenSinkResult<NodeId> {
        let closed = match &token {
            Token::TagToken(
                tag @ Tag {
                    kind: TagKind::StartTag,
                    self_closing: true,
                    ..
                },
            ) => matches!(
                tag.name,
                local_name!("title")
                    | local_name!("script")
                    | local_name!("a")
                    | local_name!("p")
                    | local_name!("div")
                    | local_name!("span")
            )
            .then(|| tag.name.clone()),
            _ => None,
        };
        let result = self.0.process_token(token, line_number);
        let Some(name) = closed else {
            return result;
        };
        let end = Token::TagToken(Tag {
            kind: TagKind::EndTag,
            name,
            self_closing: false,
            attrs: Vec::new(),
            had_duplicate_attributes: false,
        });
        match self.0.process_token(end, line_number) {
            TokenSinkResult::Script(handle) => TokenSinkResult::Script(handle),
            _ => TokenSinkResult::Continue,
        }
    }

    fn end(&self) {
        self.0.end();
    }

    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        self.0
            .adjusted_current_node_present_but_not_in_html_namespace()
    }
}

struct Sink(RefCell<Dom>);

/// An element name copied out of the borrowed arena.
#[derive(Debug)]
struct OwnedName(QualName);

impl ElemName for OwnedName {
    fn ns(&self) -> &Namespace {
        &self.0.ns
    }

    fn local_name(&self) -> &LocalName {
        &self.0.local
    }
}

impl TreeSink for Sink {
    type Handle = NodeId;
    type Output = Self;
    type ElemName<'a> = OwnedName;

    fn finish(self) -> Self {
        self
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> NodeId {
        DOCUMENT
    }

    fn elem_name<'a>(&'a self, target: &'a NodeId) -> OwnedName {
        let dom = self.0.borrow();
        let (name, _) = dom
            .element(*target)
            .expect("The parser names elements only");
        OwnedName(name.clone())
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        _flags: ElementFlags,
    ) -> NodeId {
        self.0
            .borrow_mut()
            .new_node(NodeData::Element { name, attrs })
    }

    fn create_comment(&self, text: StrTendril) -> NodeId {
        self.0
            .borrow_mut()
            .new_node(NodeData::Comment(text.to_string()))
    }

    /// HTML parsing creates no processing instructions; `<?` starts a comment.
    fn create_pi(&self, _target: StrTendril, data: StrTendril) -> NodeId {
        self.create_comment(data)
    }

    fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
        let mut dom = self.0.borrow_mut();
        match child {
            NodeOrText::AppendNode(node) => dom.append(*parent, node),
            NodeOrText::AppendText(text) => dom.append_text(*parent, &text),
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &NodeId,
        prev_element: &NodeId,
        child: NodeOrText<NodeId>,
    ) {
        if self.0.borrow().nodes[*element].parent.is_some() {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        let mut dom = self.0.borrow_mut();
        let doctype = dom.new_node(NodeData::Doctype {
            name: name.to_string(),
            public_id: public_id.to_string(),
            system_id: system_id.to_string(),
        });
        dom.append(DOCUMENT, doctype);
    }

    /// Go keeps template contents as the element's children.
    fn get_template_contents(&self, target: &NodeId) -> NodeId {
        *target
    }

    fn same_node(&self, x: &NodeId, y: &NodeId) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &NodeId, new_node: NodeOrText<NodeId>) {
        let mut dom = self.0.borrow_mut();
        let node = match new_node {
            NodeOrText::AppendNode(node) => node,
            NodeOrText::AppendText(text) => {
                let parent = dom.nodes[*sibling].parent.expect("Siblings are attached");
                let index = dom.index_in_parent(parent, *sibling);
                if let Some(previous) = index.checked_sub(1).map(|i| dom.nodes[parent].children[i])
                    && let NodeData::Text(existing) = &mut dom.nodes[previous].data
                {
                    existing.push_str(&text);
                    return;
                }
                dom.new_node(NodeData::Text(text.to_string()))
            }
        };
        dom.insert_before(*sibling, node);
    }

    fn add_attrs_if_missing(&self, target: &NodeId, attrs: Vec<Attribute>) {
        let mut dom = self.0.borrow_mut();
        if let NodeData::Element {
            attrs: existing, ..
        } = &mut dom.nodes[*target].data
        {
            for attr in attrs {
                if existing.iter().all(|present| present.name != attr.name) {
                    existing.push(attr);
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &NodeId) {
        self.0.borrow_mut().detach(*target);
    }

    fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
        self.0.borrow_mut().reparent_children(*node, *new_parent);
    }
}
