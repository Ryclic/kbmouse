use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn center(self) -> (i32, i32) {
        (
            self.x + self.width as i32 / 2,
            self.y + self.height as i32 / 2,
        )
    }

    pub fn cell(self, rows: usize, cols: usize, index: usize) -> Self {
        let row = index / cols;
        let col = index % cols;
        let x0 = self.x + (self.width as u64 * col as u64 / cols as u64) as i32;
        let x1 = self.x + (self.width as u64 * (col + 1) as u64 / cols as u64) as i32;
        let y0 = self.y + (self.height as u64 * row as u64 / rows as u64) as i32;
        let y1 = self.y + (self.height as u64 * (row + 1) as u64 / rows as u64) as i32;
        Self {
            x: x0,
            y: y0,
            width: (x1 - x0) as u32,
            height: (y1 - y0) as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_tile_negative_origin_rect() {
        let rect = Rect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            rect.cell(2, 2, 0),
            Rect {
                x: -1920,
                y: 0,
                width: 960,
                height: 540
            }
        );
        assert_eq!(rect.cell(2, 2, 3).center(), (-480, 810));
    }

    #[test]
    fn uneven_cells_reach_final_edge() {
        let rect = Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        let last = rect.cell(3, 3, 8);
        assert_eq!(last.x + last.width as i32, 10);
        assert_eq!(last.y + last.height as i32, 10);
    }
}
