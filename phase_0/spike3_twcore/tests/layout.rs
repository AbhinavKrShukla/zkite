//! The point of the spike: the styles twcore produces are fed straight into taffy and
//! lay out correctly. No conversion step, no intermediate representation.

use taffy::prelude::*;
use twcore::{NodeState, Resolver};

struct App {
    tree: TaffyTree<()>,
    res: Resolver,
}

impl App {
    fn new() -> App {
        App { tree: TaffyTree::new(), res: Resolver::new() }
    }

    fn node(&mut self, classes: &str, children: &[NodeId]) -> NodeId {
        let cs = self.res.resolve(classes, NodeState::default());
        assert!(self.res.unknown(classes).is_empty(), "bad classes: {classes}");
        self.tree.new_with_children(cs.layout.clone(), children).unwrap()
    }

    fn run(&mut self, root: NodeId, w: f32, h: f32) {
        self.tree
            .compute_layout(
                root,
                Size { width: AvailableSpace::Definite(w), height: AvailableSpace::Definite(h) },
            )
            .unwrap();
    }

    fn rect(&self, n: NodeId) -> (f32, f32, f32, f32) {
        let l = self.tree.layout(n).unwrap();
        (l.location.x, l.location.y, l.size.width, l.size.height)
    }
}

/// The counter app from plan1.md §8, laid out on a 400x800 screen.
#[test]
fn counter_app_centers_its_children() {
    let mut app = App::new();
    let text = app.node("w-[100px] h-[80px]", &[]);
    let button = app.node("w-[160px] h-[48px]", &[]);
    let column = app.node(
        "flex flex-col flex-1 items-center justify-center gap-6 bg-slate-950",
        &[text, button],
    );
    let screen = app.node("flex flex-col w-full h-full", &[column]);
    app.run(screen, 400.0, 800.0);

    // flex-1 makes the column fill the screen.
    assert_eq!(app.rect(column), (0.0, 0.0, 400.0, 800.0));

    // justify-center on a column axis: content block is 80 + 24 gap + 48 = 152 tall,
    // so it starts at (800 - 152) / 2 = 324.
    let (tx, ty, tw, th) = app.rect(text);
    assert_eq!((tw, th), (100.0, 80.0));
    assert_eq!(ty, 324.0);
    assert_eq!(tx, 150.0, "items-center should centre horizontally");

    let (bx, by, bw, _) = app.rect(button);
    assert_eq!(by, 324.0 + 80.0 + 24.0, "gap-6 is 24dp");
    assert_eq!(bx, (400.0 - bw) / 2.0);
}

#[test]
fn padding_shrinks_the_content_box() {
    let mut app = App::new();
    let child = app.node("flex-1 h-full", &[]);
    let card = app.node("flex w-[200px] h-[100px] p-4", &[child]);
    app.run(card, 200.0, 100.0);
    assert_eq!(app.rect(child), (16.0, 16.0, 168.0, 68.0));
}

#[test]
fn border_width_participates_in_layout() {
    let mut app = App::new();
    let child = app.node("flex-1 h-full", &[]);
    let card = app.node("flex w-[100px] h-[50px] border-2 p-2", &[child]);
    app.run(card, 100.0, 50.0);
    // 2dp border + 8dp padding on each side.
    assert_eq!(app.rect(child), (10.0, 10.0, 80.0, 30.0));
}

#[test]
fn justify_between_pushes_a_row_apart() {
    let mut app = App::new();
    let left = app.node("w-[40px] h-[20px]", &[]);
    let right = app.node("w-[60px] h-[20px]", &[]);
    let row = app.node("flex flex-row justify-between items-center w-[300px] h-[40px]", &[left, right]);
    app.run(row, 300.0, 40.0);
    assert_eq!(app.rect(left).0, 0.0);
    assert_eq!(app.rect(right).0, 240.0);
    assert_eq!(app.rect(left).1, 10.0, "items-center on a row centres vertically");
}

#[test]
fn absolute_inset_zero_fills_the_parent() {
    let mut app = App::new();
    let overlay = app.node("absolute inset-0", &[]);
    let parent = app.node("relative flex w-[200px] h-[120px]", &[overlay]);
    app.run(parent, 200.0, 120.0);
    assert_eq!(app.rect(overlay), (0.0, 0.0, 200.0, 120.0));
}

#[test]
fn grid_cols_three_splits_evenly() {
    let mut app = App::new();
    let cells: Vec<NodeId> = (0..3).map(|_| app.node("h-[50px]", &[])).collect();
    let grid = app.node("grid grid-cols-3 gap-4 w-[332px]", &cells);
    app.run(grid, 332.0, 50.0);
    // 332 = 3 * 100 + 2 * 16 gap.
    for (i, c) in cells.iter().enumerate() {
        let (x, _, w, _) = app.rect(*c);
        assert_eq!(w, 100.0, "cell {i} width");
        assert_eq!(x, i as f32 * 116.0, "cell {i} offset");
    }
}

#[test]
fn percentage_widths_resolve_against_the_parent() {
    let mut app = App::new();
    let half = app.node("w-1/2 h-full", &[]);
    let parent = app.node("flex w-[400px] h-[100px]", &[half]);
    app.run(parent, 400.0, 100.0);
    assert_eq!(app.rect(half).2, 200.0);
}

#[test]
fn responsive_class_changes_the_computed_layout() {
    use twcore::NodeState;
    let mut res = Resolver::new();
    let cls = "flex flex-col md:flex-row w-full h-full";
    let mut check = |width: f32, expect_row: bool| {
        let parent_style = res.resolve(cls, NodeState { viewport_width: width, ..Default::default() });
        let child_style = res.resolve("w-[50px] h-[50px]", NodeState::default());
        let mut tree: TaffyTree<()> = TaffyTree::new();
        let a = tree.new_leaf(child_style.layout.clone()).unwrap();
        let b = tree.new_leaf(child_style.layout.clone()).unwrap();
        let root = tree.new_with_children(parent_style.layout.clone(), &[a, b]).unwrap();
        tree.compute_layout(
            root,
            Size {
                width: AvailableSpace::Definite(width),
                height: AvailableSpace::Definite(200.0),
            },
        )
        .unwrap();
        let sec = tree.layout(b).unwrap().location;
        if expect_row {
            assert_eq!((sec.x, sec.y), (50.0, 0.0), "at {width}dp children should sit side by side");
        } else {
            assert_eq!((sec.x, sec.y), (0.0, 50.0), "at {width}dp children should stack");
        }
    };
    check(400.0, false);
    check(900.0, true);
}
