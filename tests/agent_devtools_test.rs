use gpui::{div, prelude::*, px, rgb, size};

#[gpui::test]
fn test_agent_devtools_dump_tree_and_locator(cx: &mut gpui::TestAppContext) {
    let cx = cx.add_empty_window();

    // 1. Renderiza uma view de teste contendo dois botões com debug_selector
    cx.draw(gpui::Point::default(), size(px(400.), px(300.)), |_, _| {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .id("btn-primary")
                    .debug_selector(|| "btn-confirm".to_string())
                    .w(px(120.))
                    .h(px(40.))
                    .bg(rgb(0x10b981))
                    .child("Confirmar"),
            )
            .child(
                div()
                    .id("btn-secondary")
                    .debug_selector(|| "btn-cancel".to_string())
                    .w(px(100.))
                    .h(px(32.))
                    .bg(rgb(0xef4444))
                    .child("Cancelar"),
            )
    });

    // 2. Dump da árvore semântica
    let tree = cx.dump_ui_tree();
    assert_eq!(tree.total_elements, 2);

    let confirm = tree.find("btn-confirm").expect("btn-confirm encontrado");
    assert_eq!(confirm.width, 120.0);
    assert_eq!(confirm.height, 40.0);
    assert!(confirm.is_visible);

    let cancel = tree.find("btn-cancel").expect("btn-cancel encontrado");
    assert_eq!(cancel.width, 100.0);
    assert_eq!(cancel.height, 32.0);

    // 3. Locator semântico
    let mut locator = cx.locator("btn-confirm");
    assert!(locator.is_visible());
    assert!(locator.click().is_ok());

    // 4. Linter de layout
    let lints = cx.lint_layout();
    assert!(lints.is_empty(), "Nenhum elemento colapsou para 0px");
}
