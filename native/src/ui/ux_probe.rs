//! Observations for the existing opt-in, isolated desktop harness.
use super::*;
pub(super) fn state(ui: &Ui) -> Value {
    let root = ui.window.clone().upcast::<gtk::Widget>();
    let mut controls = vec![];
    let mut scrolls = vec![];
    let mut art = vec![];
    fn walk(
        w: &gtk::Widget,
        window: &gtk::Widget,
        controls: &mut Vec<Value>,
        scrolls: &mut Vec<Value>,
        art: &mut Vec<Value>,
    ) {
        let bounds = w
            .compute_bounds(window)
            .map(|b| (b.x(), b.y(), b.width(), b.height()));
        if w.is_mapped()
            && (w.has_css_class("media-link")
                || w.has_css_class("media-card")
                || w.has_css_class("card-play")
                || w.has_css_class("resize-line")
                || matches!(
                    w.widget_name().as_str(),
                    "home-search-group" | "home-button" | "autoscroll-marker" | "media-card-cover"
                ))
        {
            controls.push(json!({"name":w.widget_name().as_str(),"classes":w.css_classes().iter().map(|s|s.as_str()).collect::<Vec<_>>(),"text":w.downcast_ref::<gtk::Label>().map(|l|l.text().to_string()),"rect":bounds,"focusable":w.is_focusable(),"can_target":w.can_target(),"color":w.color().to_string()}));
        }
        if w.is_mapped() {
            if let Some(sc) = w.downcast_ref::<gtk::ScrolledWindow>() {
                let b = sc.vscrollbar();
                scrolls.push(json!({"rect":bounds,"bar_rect":b.compute_bounds(window).map(|b|(b.x(),b.y(),b.width(),b.height())),"value":sc.vadjustment().value(),"upper":sc.vadjustment().upper(),"page_size":sc.vadjustment().page_size()}));
            }
        }
        if w.is_mapped() && w.widget_name().starts_with("queue-row-") {
            let picture = child(w, 0)
                .downcast::<gtk::Overlay>()
                .ok()
                .and_then(|f| f.last_child())
                .and_downcast::<gtk::Picture>();
            art.push(json!({"row":w.widget_name().as_str(),"row_id":w.as_ptr() as usize,"rect":bounds,"ready":picture.as_ref().is_some_and(|p|p.paintable().is_some()),"texture_id":picture.and_then(|p|p.paintable()).map(|p|p.as_ptr() as usize),"boundary":w.has_css_class("source-start")}));
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            walk(&c, window, controls, scrolls, art);
            child = c.next_sibling();
        }
    }
    walk(&root, &root, &mut controls, &mut scrolls, &mut art);
    json!({"controls":controls,"scrolls":scrolls,"queue_art":art,"queue_model_id":ui.queue_model.borrow().as_ref().map(|m|m.as_ptr() as usize),"main_pane":ui.content_list.parent().and_then(|w|w.parent()).and_then(|w|w.compute_bounds(&root)).map(|b|(b.x(),b.y(),b.width(),b.height())),"sidebar":ui.sidebar_widget.compute_bounds(&root).map(|b|(b.x(),b.y(),b.width(),b.height())),"right_pane":ui.right_widget.compute_bounds(&root).map(|b|(b.x(),b.y(),b.width(),b.height()))})
}
