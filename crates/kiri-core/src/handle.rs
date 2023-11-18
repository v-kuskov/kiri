// Copyright (C) 2023 Vladimir Kuskov

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

use std::{hash::Hash, marker::PhantomData};

const DEFAULT_SPACE: usize = 4096;
const GENERATION_BITS: u32 = 12;
const INDEX_BITS: u32 = 32 - GENERATION_BITS;
const INDEX_MASK: u32 = (1 << INDEX_BITS) - 1;
const GENERATION_MASK: u32 = u32::MAX - INDEX_MASK;
const MAX_INDEX: u32 = (1 << INDEX_BITS) - 1;
const MAX_GENERATION: u32 = 1 << GENERATION_BITS;

#[derive(Debug)]
pub struct Handle<T, U> {
    data: u32,
    _phantom1: PhantomData<T>,
    _phantom2: PhantomData<U>,
}

unsafe impl<T, U> Send for Handle<T, U> {}
unsafe impl<T, U> Sync for Handle<T, U> {}

#[allow(clippy::non_canonical_clone_impl)]
impl<T, U> Clone for Handle<T, U> {
    fn clone(&self) -> Self {
        Self {
            data: self.data,
            _phantom1: PhantomData,
            _phantom2: PhantomData,
        }
    }
}

impl<T, U> Copy for Handle<T, U> where T: Copy {}

impl<T, U> PartialEq for Handle<T, U> {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

impl<T, U> Eq for Handle<T, U> where T: Copy {}

impl<T, U> Hash for Handle<T, U> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.data.hash(state);
    }
}

impl<T, U> Handle<T, U> {
    pub fn new(index: u32, generation: u32) -> Self {
        assert!(index < MAX_INDEX);
        assert!(generation < MAX_GENERATION);
        Self {
            data: (generation << INDEX_BITS) | index,
            _phantom1: PhantomData,
            _phantom2: PhantomData,
        }
    }

    pub fn invalid() -> Self {
        Self {
            data: u32::MAX,
            _phantom1: PhantomData,
            _phantom2: PhantomData,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.data != u32::MAX
    }

    pub fn index(&self) -> u32 {
        self.data & INDEX_MASK
    }

    pub fn generation(&self) -> u32 {
        (self.data & GENERATION_MASK) >> INDEX_BITS
    }
}

impl<T, U> Default for Handle<T, U> {
    fn default() -> Self {
        Self::invalid()
    }
}

#[derive(Debug)]
pub struct Pool<T, U> {
    hot: Vec<Option<T>>,
    cold: Vec<Option<U>>,
    generations: Vec<u32>,
    empty: Vec<u32>,
}

impl<T, U> Pool<T, U> {
    pub fn new() -> Self {
        Self {
            hot: Vec::with_capacity(DEFAULT_SPACE),
            cold: Vec::with_capacity(DEFAULT_SPACE),
            generations: Vec::with_capacity(DEFAULT_SPACE),
            empty: Vec::with_capacity(DEFAULT_SPACE),
        }
    }

    pub fn push(&mut self, hot: T, cold: U) -> Handle<T, U> {
        if let Some(slot) = self.empty.pop() {
            self.hot[slot as usize] = Some(hot);
            self.cold[slot as usize] = Some(cold);
            Handle::new(slot, self.generations[slot as usize])
        } else {
            let index = self.generations.len();
            if index == u32::MAX as _ {
                panic!("Too many items in HandleContainer.");
            }
            self.generations.push(0);
            self.hot.push(Some(hot));
            self.cold.push(Some(cold));
            assert_eq!(self.generations.len(), self.hot.len());
            Handle::new(index as u32, 0)
        }
    }

    pub fn get(&self, handle: Handle<T, U>) -> Option<(&T, &U)> {
        if self.is_handle_valid(&handle) {
            let index = handle.index() as usize;
            Some((
                self.hot[index].as_ref().unwrap(),
                self.cold[index].as_ref().unwrap(),
            ))
        } else {
            None
        }
    }

    pub fn get_hot(&self, handle: Handle<T, U>) -> Option<&T> {
        if self.is_handle_valid(&handle) {
            let index = handle.index() as usize;
            Some(self.hot[index].as_ref().unwrap())
        } else {
            None
        }
    }

    pub fn get_cold(&self, handle: Handle<T, U>) -> Option<&U> {
        if self.is_handle_valid(&handle) {
            let index = handle.index() as usize;
            Some(self.cold[index].as_ref().unwrap())
        } else {
            None
        }
    }

    pub fn get_hot_mut(&mut self, handle: Handle<T, U>) -> Option<&mut T> {
        if self.is_handle_valid(&handle) {
            let index = handle.index() as usize;
            Some(self.hot[index].as_mut().unwrap())
        } else {
            None
        }
    }

    pub fn get_cold_mut(&mut self, handle: Handle<T, U>) -> Option<&mut U> {
        if self.is_handle_valid(&handle) {
            let index = handle.index() as usize;
            Some(self.cold[index].as_mut().unwrap())
        } else {
            None
        }
    }

    pub fn replace(&mut self, handle: Handle<T, U>, hot: T, cold: U) -> Option<(T, U)> {
        if self.is_handle_valid(&handle) {
            let index = handle.index() as usize;

            Some((
                self.hot[index].replace(hot).unwrap(),
                self.cold[index].replace(cold).unwrap(),
            ))
        } else {
            None
        }
    }

    pub fn replace_hot(&mut self, handle: Handle<T, U>, hot: T) -> Option<T> {
        if self.is_handle_valid(&handle) {
            let index = handle.index() as usize;

            Some(self.hot[index].replace(hot).unwrap())
        } else {
            None
        }
    }

    pub fn replace_cold(&mut self, handle: Handle<T, U>, cold: U) -> Option<U> {
        if self.is_handle_valid(&handle) {
            let index = handle.index() as usize;

            Some(self.cold[index].replace(cold).unwrap())
        } else {
            None
        }
    }

    pub fn remove(&mut self, handle: Handle<T, U>) -> Option<(T, U)> {
        if self.is_handle_valid(&handle) {
            let index = handle.index() as usize;
            self.generations[index] = self.generations[index].wrapping_add(1) % MAX_GENERATION;
            self.empty.push(index as _);
            return Some((
                self.hot[index].take().unwrap(),
                self.cold[index].take().unwrap(),
            ));
        }

        None
    }

    pub fn is_handle_valid(&self, handle: &Handle<T, U>) -> bool {
        let index = handle.index() as usize;
        index < self.generations.len() && self.generations[index] == handle.generation()
    }

    pub fn iter(&self) -> Iter<T, U> {
        Iter {
            container: self,
            current: 0,
        }
    }

    pub fn drain(&mut self) -> Drain<T, U> {
        Drain {
            hot: std::mem::take(&mut self.hot),
            cold: std::mem::take(&mut self.cold),
            current: 0,
        }
    }
}

impl<T, U> Default for Pool<T, U> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct Iter<'a, T, U> {
    container: &'a Pool<T, U>,
    current: usize,
}

pub struct Drain<T, U> {
    hot: Vec<Option<T>>,
    cold: Vec<Option<U>>,
    current: usize,
}

impl<'a, T, U> Iterator for Iter<'a, T, U> {
    type Item = (&'a T, &'a U);

    fn next(&mut self) -> Option<Self::Item> {
        while self.current != self.container.hot.len() && self.container.hot[self.current].is_none()
        {
            self.current += 1;
        }
        if self.current == self.container.hot.len() {
            return None;
        }
        let result = Some((
            self.container.hot[self.current].as_ref().unwrap(),
            self.container.cold[self.current].as_ref().unwrap(),
        ));
        self.current += 1;

        result
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let size = self.container.hot.len() - self.container.empty.len() - self.current;

        (size, Some(size))
    }
}

impl<T, U> Iterator for Drain<T, U> {
    type Item = (T, U);

    fn next(&mut self) -> Option<Self::Item> {
        while self.current != self.hot.len() && self.hot[self.current].is_none() {
            self.current += 1;
        }
        if self.current == self.hot.len() {
            return None;
        }
        let result = Some((
            self.hot[self.current].take().unwrap(),
            self.cold[self.current].take().unwrap(),
        ));
        self.current += 1;

        result
    }
}

#[cfg(test)]
mod test {
    use crate::{Handle, Pool};

    #[test]
    fn handle() {
        let handle = Handle::<(), ()>::new(100, 10);
        assert_eq!(100, handle.index());
        assert_eq!(10, handle.generation());
    }

    #[test]
    fn handle_container_push_get() {
        let mut container = Pool::<u32, i32>::new();
        let handle1 = container.push(1, -1);
        let handle2 = container.push(2, -2);
        let handle3 = container.push(3, -3);
        assert_eq!(Some((&1, &-1)), container.get(handle1));
        assert_eq!(Some((&2, &-2)), container.get(handle2));
        assert_eq!(Some((&3, &-3)), container.get(handle3));
        assert_eq!(Some(&2), container.get_hot(handle2));
        assert_eq!(Some(&-3), container.get_cold(handle3));
    }

    #[test]
    fn reuse_slot() {
        let mut container = Pool::<u32, i32>::new();
        let handle = container.push(1, -1);
        container.remove(handle);
        let handle = container.push(2, -2);
        assert_eq!(1, handle.generation());
        assert_eq!(0, handle.index());
        assert_eq!(Some((&2, &-2)), container.get(handle));
    }

    #[test]
    fn old_handle_returns_none() {
        let mut container = Pool::<u32, i32>::new();
        let handle1 = container.push(1, -1);
        assert_eq!(Some((1, -1)), container.remove(handle1));
        let handle2 = container.push(2, -2);
        assert_eq!(None, container.get(handle1));
        assert_eq!(Some((&2, &-2)), container.get(handle2));
    }

    #[test]
    fn mutate_by_handle() {
        let mut container = Pool::<u32, i32>::new();
        let handle = container.push(1, -1);
        assert_eq!(Some((&1, &-1)), container.get(handle));
        assert_eq!(Some((1, -1)), container.replace(handle, 2, -2));
        assert_eq!(Some((&2, &-2)), container.get(handle));
        assert_eq!(Some(2), container.replace_hot(handle, 3));
        assert_eq!(Some(-2), container.replace_cold(handle, -3));
        assert_eq!(Some((&3, &-3)), container.get(handle));
    }

    #[test]
    fn iterate_empty() {
        let container = Pool::<u32, i32>::new();
        let cont = container.iter().map(|(x, y)| (*x, *y)).collect::<Vec<_>>();
        assert!(cont.is_empty());
    }

    #[test]
    fn iterate_full() {
        let mut container = Pool::<u32, i32>::new();
        container.push(1, -1);
        container.push(2, -2);
        container.push(3, -3);
        let cont = container.iter().map(|(x, y)| (*x, *y)).collect::<Vec<_>>();
        assert_eq!([(1u32, -1i32), (2, -2), (3, -3)].to_vec(), cont);
    }

    #[test]
    fn iterate_hole() {
        let mut container = Pool::<u32, i32>::new();
        container.push(1, -1);
        let handle = container.push(2, -2);
        container.push(3, -3);
        container.remove(handle);
        let cont = container.iter().map(|(x, y)| (*x, *y)).collect::<Vec<_>>();
        assert_eq!([(1u32, -1i32), (3, -3)].to_vec(), cont);
    }

    #[test]
    fn drain() {
        let mut container = Pool::<u32, i32>::new();
        container.push(1, -1);
        container.push(2, -2);
        container.push(3, -3);

        let cont = container.drain().collect::<Vec<_>>();
        assert_eq!([(1u32, -1i32), (2, -2), (3, -3)].to_vec(), cont);
        assert!(container.iter().collect::<Vec<_>>().is_empty());
    }
}
