use crate::file_icons::{Request, Result, Worker};
use std::{borrow::Cow, cell::OnceCell, collections::HashMap};
use windows_sys::Win32::UI::Controls::{ImageList_ReplaceIcon, HIMAGELIST};

pub const GENERIC_FILE: i32 = 1;
const IMAGE_BUDGET: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy)]
enum State {
    Pending,
    Image(i32),
}

struct Cached {
    state: State,
    stamp: u64,
    viewport: u64,
}

#[derive(Default)]
struct Images {
    by_type: HashMap<Vec<u16>, Cached>,
    unused: Vec<i32>,
    generation: u64,
    viewport: u64,
    deferred: bool,
    viewport_full: bool,
}

impl Images {
    fn oldest_outside_viewport(&self) -> Option<&Vec<u16>> {
        self.by_type
            .iter()
            .filter(|(_, cached)| {
                cached.viewport != self.viewport && matches!(cached.state, State::Image(_))
            })
            .min_by_key(|(_, cached)| cached.stamp)
            .map(|(key, _)| key)
    }
}

#[derive(Default)]
pub struct FileIconCache {
    images: [Images; 2],
    worker: OnceCell<Option<Worker>>,
    stamp: u64,
}

impl FileIconCache {
    pub fn reset(&mut self, slot: usize) {
        let images = &mut self.images[slot];
        images.generation = images.generation.wrapping_add(1);
        images.by_type.clear();
        images.unused.clear();
        images.deferred = false;
        images.viewport_full = false;
    }

    pub fn viewport_changed(&mut self, slot: usize) -> bool {
        let images = &mut self.images[slot];
        images.viewport = images.viewport.wrapping_add(1);
        std::mem::take(&mut images.viewport_full)
    }

    pub fn image(
        &mut self,
        slot: usize,
        side: i32,
        extension: &[u16],
        start: impl FnOnce() -> Option<Worker>,
    ) -> i32 {
        let mut scratch = [0; 32];
        let key = type_key(extension, &mut scratch);
        if key.is_empty() {
            return GENERIC_FILE;
        }
        self.stamp = self.stamp.wrapping_add(1);
        let images = &mut self.images[slot];
        if let Some(cached) = images.by_type.get_mut(key.as_ref()) {
            cached.stamp = self.stamp;
            cached.viewport = images.viewport;
            return match cached.state {
                State::Pending => GENERIC_FILE,
                State::Image(index) => index,
            };
        }

        let Some(worker) = self.worker.get_or_init(start) else {
            return GENERIC_FILE;
        };
        // Large icons consume more bitmap space per slot than Details icons.
        let bytes = (side.max(1) as usize).saturating_pow(2).saturating_mul(4);
        let capacity = (IMAGE_BUDGET / bytes).clamp(8, 512);
        let evicted = if images.by_type.len() >= capacity {
            // Redrawing resolved icons must not evict other icons still in the same viewport.
            let Some(oldest) = images.oldest_outside_viewport() else {
                images.viewport_full = true;
                return GENERIC_FILE;
            };
            Some(oldest.clone())
        } else {
            None
        };
        let request = Request {
            slot,
            generation: images.generation,
            key: key.to_vec(),
            side,
        };
        if worker.request(request) {
            if let Some(oldest) = evicted {
                if let Some(Cached {
                    state: State::Image(index),
                    ..
                }) = images.by_type.remove(&oldest)
                {
                    if index != GENERIC_FILE {
                        images.unused.push(index);
                    }
                }
            }
            images.by_type.insert(
                key.into_owned(),
                Cached {
                    state: State::Pending,
                    stamp: self.stamp,
                    viewport: images.viewport,
                },
            );
        } else {
            images.deferred = true;
        }
        GENERIC_FILE
    }

    pub fn apply(&mut self, result: Result, lists: &[HIMAGELIST; 2]) -> bool {
        let Request {
            slot,
            generation,
            key,
            ..
        } = result.request;
        let images = &mut self.images[slot];
        if generation != images.generation {
            return false;
        }
        let Some(cached) = images.by_type.get_mut(&key) else {
            return false;
        };
        let mut index = GENERIC_FILE;
        if let Some(icon) = result.icon.filter(|_| lists[slot] != 0) {
            let reuse = images.unused.last().copied().unwrap_or(-1);
            let added = unsafe { ImageList_ReplaceIcon(lists[slot], reuse, icon.handle()) };
            if added >= 0 {
                if reuse >= 0 {
                    images.unused.pop();
                }
                index = added;
            }
        }
        cached.state = State::Image(index);
        index != GENERIC_FILE
    }

    pub fn needs_retry(&mut self, slot: usize) -> bool {
        std::mem::take(&mut self.images[slot].deferred)
    }
}

fn type_key<'a>(extension: &'a [u16], scratch: &'a mut [u16; 32]) -> Cow<'a, [u16]> {
    let suffix = extension.strip_suffix(&[0]).unwrap_or(extension);
    let is_uppercase = |unit| (u16::from(b'A')..=u16::from(b'Z')).contains(&unit);
    if !suffix.iter().copied().any(is_uppercase) {
        return Cow::Borrowed(suffix);
    }
    let fold = |unit| if is_uppercase(unit) { unit + 32 } else { unit };
    if suffix.len() <= scratch.len() {
        for (destination, source) in scratch.iter_mut().zip(suffix.iter().copied()) {
            *destination = fold(source);
        }
        Cow::Borrowed(&scratch[..suffix.len()])
    } else {
        Cow::Owned(suffix.iter().copied().map(fold).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_lookup_uses_one_key_for_matching_ascii_case() {
        let mut scratch = [0; 32];
        assert_eq!(
            type_key(&[80, 100, 70, 0], &mut scratch).as_ref(),
            &[112, 100, 102]
        );
        assert_eq!(
            type_key(&[109, 99, 100, 120, 0], &mut scratch).as_ref(),
            &[109, 99, 100, 120]
        );
        assert!(type_key(&[0], &mut scratch).is_empty());
        let oversized = [65; 33];
        assert_eq!(type_key(&oversized, &mut scratch).as_ref(), &[97; 33]);
    }

    #[test]
    fn old_icon_result_cannot_complete_a_request_after_recreating_the_image_list() {
        let mut cache = FileIconCache::default();
        let key: Vec<_> = "pdf".encode_utf16().collect();
        let old_generation = cache.images[1].generation;
        cache.reset(1);
        cache.images[1].by_type.insert(
            key.clone(),
            Cached {
                state: State::Pending,
                stamp: 0,
                viewport: 0,
            },
        );
        assert!(!cache.apply(
            Result {
                request: Request {
                    slot: 1,
                    generation: old_generation,
                    key: key.clone(),
                    side: 112
                },
                icon: None,
            },
            &[0, 0],
        ));
        assert!(matches!(
            cache.images[1].by_type[&key].state,
            State::Pending
        ));
    }

    #[test]
    fn cache_does_not_recycle_icon_slots_until_the_viewport_changes() {
        let mut cache = FileIconCache::default();
        let key = vec![65];
        cache.images[1].by_type.insert(
            key.clone(),
            Cached {
                state: State::Image(3),
                stamp: 1,
                viewport: 0,
            },
        );
        cache.images[1].viewport_full = true;
        assert_eq!(cache.images[1].oldest_outside_viewport(), None);
        assert!(cache.viewport_changed(1));
        assert_eq!(cache.images[1].oldest_outside_viewport(), Some(&key));
    }
}
