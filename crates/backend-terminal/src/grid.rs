//! A simple row-major character grid used as the terminal raster target. Each
//! cell carries an optional color used when rendering in ANSI mode.

/// Per-cell color. `Default` means "no color" (terminal default foreground).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum CellColor {
    #[default]
    Default,
    Green,
    Red,
    /// Annotations: a third color so a note never reads as a buy or a sell.
    Note,
}

/// A `cols x rows` character grid, row-major, initialised to blank cells.
pub(crate) struct Grid {
    cols: usize,
    rows: usize,
    cells: Vec<char>,
    colors: Vec<CellColor>,
}

impl Grid {
    pub(crate) fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            cells: vec![' '; cols * rows],
            colors: vec![CellColor::Default; cols * rows],
        }
    }

    pub(crate) fn cols(&self) -> usize {
        self.cols
    }

    pub(crate) fn rows(&self) -> usize {
        self.rows
    }

    /// Write `ch` with an explicit color at `(col, row)`. Out-of-bounds writes
    /// are silently ignored so callers never have to bounds-check rounded
    /// coordinates.
    pub(crate) fn set_styled(&mut self, col: usize, row: usize, ch: char, color: CellColor) {
        if col < self.cols && row < self.rows {
            let idx = row * self.cols + col;
            self.cells[idx] = ch;
            self.colors[idx] = color;
        }
    }

    pub(crate) fn get(&self, col: usize, row: usize) -> char {
        if col < self.cols && row < self.rows {
            self.cells[row * self.cols + col]
        } else {
            ' '
        }
    }

    pub(crate) fn get_color(&self, col: usize, row: usize) -> CellColor {
        if col < self.cols && row < self.rows {
            self.colors[row * self.cols + col]
        } else {
            CellColor::Default
        }
    }

    /// Render the grid to a string, rows top-to-bottom joined by newlines.
    /// Currently used only by tests; the chart composer writes cells directly.
    #[cfg(test)]
    pub(crate) fn to_plain_string(&self) -> String {
        let mut out = String::with_capacity(self.rows * (self.cols + 1));
        for row in 0..self.rows {
            let start = row * self.cols;
            out.extend(&self.cells[start..start + self.cols]);
            if row + 1 < self.rows {
                out.push('\n');
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_grid_is_blank_with_correct_dimensions() {
        let g = Grid::new(4, 3);
        let rendered = g.to_plain_string();
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 3);
        for line in lines {
            assert_eq!(line.chars().count(), 4);
            assert!(line.chars().all(|c| c == ' '));
        }
    }

    #[test]
    fn set_then_get_round_trips() {
        let mut g = Grid::new(4, 3);
        g.set_styled(2, 1, '█', CellColor::Green);
        assert_eq!(g.get(2, 1), '█');
        assert_eq!(g.get_color(2, 1), CellColor::Green);
        assert_eq!(g.get(0, 0), ' ');
        assert_eq!(g.get_color(0, 0), CellColor::Default);
    }

    #[test]
    fn set_out_of_bounds_is_ignored() {
        let mut g = Grid::new(2, 2);
        g.set_styled(99, 99, 'x', CellColor::Red);
        g.set_styled(0, 5, 'y', CellColor::Red);
        assert_eq!(g.get(0, 0), ' ');
    }
}
