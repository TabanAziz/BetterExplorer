use better_explorer::model::Location;
use std::path::{Component, Path, Prefix};

#[derive(Debug, Eq, PartialEq)]
pub struct Placement {
    pub index: Option<usize>,
    pub width: i32,
}

pub fn segments(location: &Location, system_drive: Option<&str>) -> Vec<(String, Location)> {
    let Some(path) = location
        .directory()
        .filter(|path| !path.as_os_str().is_empty())
    else {
        return vec![(location.caption(), location.clone())];
    };

    let mut segments: Vec<_> = path
        .ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .map(|ancestor| {
            (
                folder_caption(ancestor, system_drive),
                Location::path(ancestor),
            )
        })
        .collect();
    segments.push(("This PC".into(), Location::Computer));
    segments.reverse();
    segments
}

fn folder_caption(path: &Path, system_drive: Option<&str>) -> String {
    if let Some(name) = path.file_name() {
        return name.to_string_lossy().into_owned();
    }
    if let Some(Component::Prefix(prefix)) = path.components().next() {
        match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                let drive = format!("{}:", char::from(drive));
                let is_system = system_drive.is_some_and(|system| {
                    system
                        .trim_end_matches(['\\', '/'])
                        .eq_ignore_ascii_case(&drive)
                });
                return if is_system {
                    format!("Local Disk ({drive})")
                } else {
                    drive
                };
            }
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                return format!(
                    r"\\{}\{}",
                    server.to_string_lossy(),
                    share.to_string_lossy()
                );
            }
            _ => {}
        }
    }
    path.to_string_lossy().into_owned()
}

/// Widths include the space used by each button's padding and decoration.
/// A `None` index stands for the contiguous run of hidden ancestors.
pub fn fit(
    widths: &[i32],
    available: i32,
    ellipsis_width: i32,
    minimum_current: i32,
) -> Vec<Placement> {
    if widths.is_empty() || available <= 0 {
        return Vec::new();
    }
    let width = |index: usize| widths[index].max(0);
    let full_width = widths.iter().map(|&px| i64::from(px.max(0))).sum::<i64>();
    if full_width <= i64::from(available) {
        return (0..widths.len())
            .map(|index| Placement {
                index: Some(index),
                width: width(index),
            })
            .collect();
    }

    let current = widths.len() - 1;
    let current_width = width(current);
    let ellipsis_width = ellipsis_width.max(1);
    let current_minimum = minimum_current.max(1).min(current_width);
    if current == 0 || available - current_minimum < ellipsis_width {
        return vec![Placement {
            index: Some(current),
            width: available.min(current_width),
        }];
    }

    let shown_current = (available - ellipsis_width).min(current_width);
    let mut spare = available - ellipsis_width - shown_current;
    let mut tail_start = current;
    while tail_start > 1 && width(tail_start - 1) <= spare {
        tail_start -= 1;
        spare -= width(tail_start);
    }

    let keep_root = tail_start > 1 && width(0) <= spare;
    let mut placements = Vec::with_capacity(current - tail_start + 2 + usize::from(keep_root));
    if keep_root {
        placements.push(Placement {
            index: Some(0),
            width: width(0),
        });
    }
    placements.push(Placement {
        index: None,
        width: ellipsis_width,
    });
    placements.extend((tail_start..current).map(|index| Placement {
        index: Some(index),
        width: width(index),
    }));
    placements.push(Placement {
        index: Some(current),
        width: shown_current,
    });
    placements
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_drive_has_a_label_and_each_segment_has_an_exact_target() {
        let current = Location::path(r"C:\Users\Aziz\Drawings & Plans");
        assert_eq!(
            segments(&current, Some("c:")),
            vec![
                ("This PC".into(), Location::Computer),
                ("Local Disk (C:)".into(), Location::path(r"C:\")),
                ("Users".into(), Location::path(r"C:\Users")),
                ("Aziz".into(), Location::path(r"C:\Users\Aziz")),
                ("Drawings & Plans".into(), current),
            ],
        );
    }

    #[test]
    fn unc_share_root_remains_a_navigable_target() {
        let current = Location::path(r"\\server\share\Team Folder");
        assert_eq!(
            segments(&current, Some("C:")),
            vec![
                ("This PC".into(), Location::Computer),
                (r"\\server\share".into(), Location::path(r"\\server\share\")),
                ("Team Folder".into(), current),
            ],
        );
    }

    #[test]
    fn search_uses_the_path_of_its_search_root() {
        let location = Location::Search {
            root: r"D:\Projects\Current".into(),
            query: "layout".into(),
        };
        assert_eq!(
            segments(&location, Some("C:")),
            vec![
                ("This PC".into(), Location::Computer),
                ("D:".into(), Location::path(r"D:\")),
                ("Projects".into(), Location::path(r"D:\Projects")),
                ("Current".into(), Location::path(r"D:\Projects\Current")),
            ],
        );
    }

    fn placement(index: Option<usize>, width: i32) -> Placement {
        Placement { index, width }
    }

    #[test]
    fn fitted_breadcrumbs_keep_their_full_width() {
        assert_eq!(
            fit(&[66, 101, 125, 220], 512, 29, 50),
            vec![
                placement(Some(0), 66),
                placement(Some(1), 101),
                placement(Some(2), 125),
                placement(Some(3), 220),
            ],
        );
    }

    #[test]
    fn narrow_layout_keeps_the_root_when_it_can_also_keep_the_same_tail() {
        assert_eq!(
            fit(&[40, 90, 60, 110], 230, 20, 50),
            vec![
                placement(Some(0), 40),
                placement(None, 20),
                placement(Some(2), 60),
                placement(Some(3), 110),
            ],
        );
        assert_eq!(
            fit(&[40, 90, 60, 110], 190, 20, 50),
            vec![
                placement(None, 20),
                placement(Some(2), 60),
                placement(Some(3), 110),
            ],
        );
    }

    #[test]
    fn only_the_current_segment_can_shrink_when_the_path_overflows() {
        assert_eq!(
            fit(&[66, 50, 150], 170, 20, 50),
            vec![placement(None, 20), placement(Some(2), 150)],
        );
        assert_eq!(
            fit(&[66, 50, 150], 120, 20, 50),
            vec![placement(None, 20), placement(Some(2), 100)],
        );
        assert_eq!(
            fit(&[66, 50, 150], 60, 20, 50),
            vec![placement(Some(2), 60)],
        );
    }

    #[test]
    fn deep_layout_keeps_the_tail_and_does_not_cap_the_wide_case() {
        let widths = vec![20; 128];
        assert_eq!(
            fit(&widths, 85, 25, 12),
            vec![
                placement(None, 25),
                placement(Some(125), 20),
                placement(Some(126), 20),
                placement(Some(127), 20),
            ],
        );
        assert_eq!(fit(&widths, 2560, 25, 12).len(), 128);
    }
}
