//! Vertical scrolling shared by every scrollable part of the UI: the offset, the wheel, and the thin
//! scroll bar with a draggable thumb. All values are in pixels.

#[derive(Default, Clone, Debug)]
pub struct VScroll {
    pub offset: f32,
    /// While the thumb is being dragged: where inside the thumb it was grabbed.
    drag: Option<f32>,
}

fn inside(p: (f32, f32), r: [f32; 4]) -> bool {
    p.0 >= r[0] && p.0 < r[0] + r[2] && p.1 >= r[1] && p.1 < r[1] + r[3]
}

impl VScroll {
    pub fn max(content: f32, view: f32) -> f32 {
        (content - view).max(0.0)
    }

    pub fn clamp(&mut self, content: f32, view: f32) {
        self.offset = self.offset.clamp(0.0, Self::max(content, view));
    }

    /// Wheel notches (positive = up) of `step` pixels each.
    pub fn wheel(&mut self, notches: f32, step: f32, content: f32, view: f32) {
        self.offset -= notches * step;
        self.clamp(content, view);
    }

    /// The thumb inside the track strip, if the content is taller than the view.
    pub fn thumb(&self, track: [f32; 4], content: f32, view: f32) -> Option<[f32; 4]> {
        if content <= view + 0.5 || track[3] <= 0.0 {
            return None;
        }
        let h = (track[3] * view / content).clamp(24.0f32.min(track[3]), track[3]);
        let y = track[1] + (track[3] - h) * (self.offset / Self::max(content, view));
        Some([track[0], y, track[2], h])
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// A press inside the track: grabs the thumb or pages towards the click. True when it hit the track.
    pub fn press(&mut self, p: (f32, f32), track: [f32; 4], content: f32, view: f32) -> bool {
        let Some(t) = self.thumb(track, content, view) else { return false };
        if !inside(p, track) {
            return false;
        }
        if inside(p, t) {
            self.drag = Some(p.1 - t[1]);
        } else {
            self.offset += if p.1 < t[1] { -view } else { view } * 0.9;
            self.clamp(content, view);
        }
        true
    }

    pub fn drag_to(&mut self, y: f32, track: [f32; 4], content: f32, view: f32) {
        let (Some(grab), Some(t)) = (self.drag, self.thumb(track, content, view)) else { return };
        let travel = (track[3] - t[3]).max(1.0);
        let frac = ((y - grab - track[1]) / travel).clamp(0.0, 1.0);
        self.offset = frac * Self::max(content, view);
    }

    pub fn release(&mut self) {
        self.drag = None;
    }

    /// Scrolls just enough to make the range [top, bottom] (content coordinates) visible.
    pub fn reveal(&mut self, top: f32, bottom: f32, content: f32, view: f32) {
        if top < self.offset {
            self.offset = top;
        } else if bottom > self.offset + view {
            self.offset = bottom - view;
        }
        self.clamp(content, view);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_and_thumb() {
        let mut s = VScroll::default();
        let track = [90.0, 0.0, 10.0, 100.0];
        assert!(s.thumb(track, 80.0, 100.0).is_none(), "nothing to scroll");
        s.wheel(-3.0, 20.0, 300.0, 100.0); // three notches down
        assert_eq!(s.offset, 60.0);
        s.wheel(-100.0, 20.0, 300.0, 100.0);
        assert_eq!(s.offset, 200.0, "stops at the end");
        let t = s.thumb(track, 300.0, 100.0).unwrap();
        assert!((t[3] - 33.3).abs() < 0.5 && (t[1] + t[3] - 100.0).abs() < 0.01);
    }

    #[test]
    fn dragging_the_thumb_and_paging() {
        let mut s = VScroll::default();
        let track = [90.0, 0.0, 10.0, 100.0];
        assert!(s.press((95.0, 10.0), track, 300.0, 100.0), "grabs the thumb");
        s.drag_to(40.0, track, 300.0, 100.0);
        assert!(s.offset > 80.0 && s.offset < 100.0, "{}", s.offset);
        s.release();
        s.offset = 0.0;
        assert!(s.press((95.0, 90.0), track, 300.0, 100.0));
        assert!(s.offset > 80.0 && !s.dragging(), "a click below the thumb pages down");
        assert!(!s.press((10.0, 10.0), track, 300.0, 100.0), "outside the track");
    }

    #[test]
    fn reveal_scrolls_just_enough() {
        let mut s = VScroll::default();
        s.reveal(150.0, 170.0, 400.0, 100.0);
        assert_eq!(s.offset, 70.0);
        s.reveal(80.0, 100.0, 400.0, 100.0);
        assert_eq!(s.offset, 70.0, "already visible");
        s.reveal(10.0, 30.0, 400.0, 100.0);
        assert_eq!(s.offset, 10.0);
    }
}
