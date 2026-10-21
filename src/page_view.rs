use std::cell::{Cell, RefCell};

use gtk::{gdk, glib, graphene, prelude::*, subclass::prelude::*};

use crate::region::Region;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ViewMode {
    Crop,
    #[default]
    Focus,
    Page,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PageView {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub region: Cell<Option<Region>>,
        pub mode: Cell<ViewMode>,
        pub zoom: Cell<f32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PageView {
        const NAME: &'static str = "PanelPageView";
        type Type = super::PageView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for PageView {}

    impl WidgetImpl for PageView {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(texture) = self.texture.borrow().as_ref().cloned() else {
                return;
            };
            let widget = self.obj();
            let viewport_width = widget.width() as f32;
            let viewport_height = widget.height() as f32;
            if viewport_width <= 0.0 || viewport_height <= 0.0 {
                return;
            }

            let page_width = texture.width() as f32;
            let page_height = texture.height() as f32;
            let region = self.region.get().unwrap_or(Region {
                x: 0,
                y: 0,
                width: texture.width() as u32,
                height: texture.height() as u32,
            });
            let mode = self.mode.get();
            let target = if mode == ViewMode::Page {
                Region {
                    x: 0,
                    y: 0,
                    width: texture.width() as u32,
                    height: texture.height() as u32,
                }
            } else {
                region
            };
            let padding = 32.0;
            let available_width = (viewport_width - padding * 2.0).max(1.0);
            let available_height = (viewport_height - padding * 2.0).max(1.0);
            let mut scale = (available_width / target.width as f32)
                .min(available_height / target.height as f32);
            if mode == ViewMode::Focus {
                let page_fit = (available_width / page_width).min(available_height / page_height);
                scale = scale.min(page_fit * 2.0);
            }
            let zoom = self.zoom.get();
            scale *= if zoom == 0.0 { 1.0 } else { zoom };
            let target_center_x = target.x as f32 + target.width as f32 / 2.0;
            let target_center_y = target.y as f32 + target.height as f32 / 2.0;
            let page_x = viewport_width / 2.0 - target_center_x * scale;
            let page_y = viewport_height / 2.0 - target_center_y * scale;

            snapshot.save();
            snapshot.translate(&graphene::Point::new(page_x, page_y));
            snapshot.scale(scale, scale);
            snapshot.append_texture(
                &texture,
                &graphene::Rect::new(0.0, 0.0, page_width, page_height),
            );

            snapshot.restore();
        }
    }
}

glib::wrapper! {
    pub struct PageView(ObjectSubclass<imp::PageView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PageView {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("hexpand", true)
            .property("vexpand", true)
            .build()
    }

    pub fn zoom(&self) -> f32 {
        let zoom = self.imp().zoom.get();
        if zoom == 0.0 { 1.0 } else { zoom }
    }

    pub fn set_zoom(&self, zoom: f32) {
        self.imp().zoom.set(zoom.clamp(0.5, 3.0));
        self.queue_draw();
    }

    pub fn change_zoom(&self, steps: i32) -> f32 {
        let zoom = ((self.zoom() * 10.0).round() as i32 + steps * 2).clamp(5, 30) as f32 / 10.0;
        self.imp().zoom.set(zoom);
        self.queue_draw();
        zoom
    }

    pub fn set_page(&self, texture: gdk::Texture, region: Region) {
        self.imp().texture.replace(Some(texture));
        self.imp().region.set(Some(region));
        self.queue_draw();
    }

    pub fn set_region(&self, region: Region) {
        self.imp().region.set(Some(region));
        self.queue_draw();
    }

    pub fn set_mode(&self, mode: ViewMode) {
        self.imp().mode.set(mode);
        self.queue_draw();
    }

    pub fn mode(&self) -> ViewMode {
        self.imp().mode.get()
    }
}
