#![feature(allocator_api)]

use alloc::{Global, Layout};
use ptr::NonNull;
use std::{alloc, fmt, mem, ptr};
use std::alloc::Allocator;
use std::cmp::max;
use std::fmt::{Formatter, Write};
use std::ops::Add;

pub struct GapBuffer<T, A: Allocator = Global> {
    allocator: A,
    buffer: NonNull<T>,
    buffer_capacity: usize,
    gap_start: usize,
    gap_len: usize,
}

impl<T> GapBuffer<T, Global> {
    pub fn new() -> Self {
        Self {
            allocator: Global,
            buffer: NonNull::dangling(),
            buffer_capacity: 0,
            gap_start: 0,
            gap_len: 0,
        }
    }
}

impl<T, A: Allocator> GapBuffer<T, A> {
    const MIN_NON_ZERO_CAP: usize = if mem::size_of::<T>() == 1 {
        8
    } else if mem::size_of::<T>() <= 1024 {
        4
    } else {
        1
    };

    pub fn new_with_allocator(allocator: A) -> Self {
        Self {
            allocator,
            buffer: NonNull::dangling(),
            buffer_capacity: 0,
            gap_start: 0,
            gap_len: 0,
        }
    }

    pub fn get(&self, i: usize) -> Option<&T> {
        if i >= self.len() {
            None
        } else {
            let offset = if i < self.gap_start {
                i
            } else {
                i + self.gap_start + self.gap_len
            };
            Some(unsafe { &*self.buffer.as_ptr().add(offset) })
        }
    }

    pub fn push(&mut self, value: T) {
        self.insert(self.len(), value)
    }

    pub fn insert(&mut self, i: usize, value: T) {
        if i > self.len() {
            panic!("Index out of bound for {:?} of buffer's size {:?}", i, self.len())
        }
        self.gap_move_to(i);
        self.gap_ensure_size(1);
        unsafe { self.buffer.as_ptr().add(self.gap_start).write(value) }
        self.gap_start += 1;
        self.gap_len -= 1;
    }

    pub fn delete(&mut self, i: usize) -> T {
        if i >= self.len() {
            panic!("Index out of bound for {:?} of buffer's size {:?}", i, self.len())
        }
        self.gap_move_to(i);
        self.gap_len += 1;
        unsafe { self.buffer.as_ptr().add(self.gap_start).add(self.gap_len - 1).read() }
    }

    pub fn len(&self) -> usize {
        self.buffer_capacity - self.gap_len
    }

    pub fn capacity(&self) -> usize {
        self.buffer_capacity
    }

    fn gap_move_to(&mut self, i: usize) {
        if i != self.gap_start {
            let buffer = self.buffer.as_ptr();
            if i < self.gap_start {
                unsafe {
                    let src = buffer.add(i);
                    let dst = buffer.add(i).add(self.gap_len);
                    ptr::copy(src, dst, self.gap_start - i)
                }
            } else {
                unsafe {
                    let src = buffer.add(self.gap_start).add(self.gap_len);
                    let dst = buffer.add(self.gap_start);
                    ptr::copy(src, dst, i - self.gap_start)
                }
            }
            self.gap_start = i;
        }
    }

    fn gap_ensure_size(&mut self, size: usize) {
        if self.gap_len < size {
            let new_capacity = max(self.buffer_capacity * 2, size);
            let new_capacity = max(new_capacity, GapBuffer::<T>::MIN_NON_ZERO_CAP);
            let new_gap_len = new_capacity - self.len();
            let new_layout = Layout::array::<T>(new_capacity).unwrap();
            let new_buffer = if let Some(old_layout) = self.buffer_layout() {
                unsafe {
                    self.allocator.grow(self.buffer.cast(), old_layout, new_layout).unwrap().cast()
                }
            } else {
                self.allocator.allocate(new_layout).unwrap().cast()
            };
            self.buffer = new_buffer;
            self.buffer_capacity = new_capacity;
            self.gap_len = new_gap_len;
        }
    }

    fn buffer_extend_from_vec(&mut self, vec: Vec<T>) {
        self.gap_ensure_size(vec.len());
        for value in vec {
            self.push(value)
        }
    }

    fn buffer_layout(&self) -> Option<Layout> {
        if self.buffer_capacity == 0 {
            None
        } else {
            Some(Layout::array::<T>(self.buffer_capacity).unwrap())
        }
    }
}

impl<T, A: Allocator> Drop for GapBuffer<T, A> {
    fn drop(&mut self) {
        unsafe {
            let ptr = self.buffer.as_ptr();
            let len = self.gap_start;
            ptr::drop_in_place(ptr::slice_from_raw_parts_mut(ptr, len));

            let ptr = ptr.add(self.gap_start).add(self.gap_len);
            let len = self.buffer_capacity - self.gap_start - self.gap_len;
            ptr::drop_in_place(ptr::slice_from_raw_parts_mut(ptr, len));

            if let Some(layout) = self.buffer_layout() {
                self.allocator.deallocate(self.buffer.cast(), layout);
            }
        }
    }
}

impl<T> From<GapBuffer<T>> for Box<[T]> {
    fn from(value: GapBuffer<T>) -> Self {
        let mut value = value;
        let mut vec = Vec::<T>::with_capacity(value.len());
        for _ in 0..value.len() {
            vec.push(value.delete(0));
        }
        vec.into_boxed_slice()
    }
}

impl<T> From<Box<[T]>> for GapBuffer<T> {
    fn from(value: Box<[T]>) -> Self {
        let mut buffer = GapBuffer::<T>::new();
        buffer.buffer_extend_from_vec(value.into_vec());
        buffer
    }
}

impl<T: fmt::Debug> fmt::Debug for GapBuffer<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_char('[')?;
        let ptr = self.buffer.as_ptr();
        for i in 0..self.gap_start {
            if i != 0 {
                f.write_str(", ")?;
            }
            unsafe { &*ptr.add(i) }.fmt(f)?;
        }
        if self.gap_len > 0 {
            f.write_str(", [...]")?;
        }
        for i in self.gap_start.add(self.gap_len)..self.buffer_capacity {
            f.write_str(", ")?;
            unsafe { &*ptr.add(i) }.fmt(f)?;
        }
        f.write_char(']')
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use crate::GapBuffer;

    #[test]
    fn gap_buffer_with_capacity_8() {
        let mut buf = GapBuffer::<u8>::new();
        buf.gap_ensure_size(1);
        assert_eq!(buf.gap_len, GapBuffer::<u8>::MIN_NON_ZERO_CAP);
    }

    #[test]
    fn gap_buffer_with_capacity_4() {
        let mut buf = GapBuffer::<[u8; 1024]>::new();
        buf.gap_ensure_size(1);
        assert_eq!(buf.gap_len, GapBuffer::<[u8; 1024]>::MIN_NON_ZERO_CAP);
    }

    #[test]
    fn gap_buffer_with_capacity_1() {
        let mut buf = GapBuffer::<[u8; 1025]>::new();
        buf.gap_ensure_size(1);
        assert_eq!(buf.gap_len, GapBuffer::<[u8; 1025]>::MIN_NON_ZERO_CAP);
    }

    #[test]
    fn gap_buffer_get() {
        let mut buf = GapBuffer::<u8>::new();
        buf.push(1);
        buf.push(2);
        buf.push(3);
        assert_eq!(Some(&1u8), buf.get(0));
        assert_eq!(Some(&2u8), buf.get(1));
        assert_eq!(Some(&3u8), buf.get(2));
    }

    #[test]
    fn gap_buffer_push() {
        let mut buf = GapBuffer::<u8>::new();
        buf.push(1);
        buf.push(2);
        buf.push(3);
        let boxed = Box::<[u8]>::from(buf);
        assert_eq!(&[1, 2, 3], boxed.as_ref());
    }

    #[test]
    fn gap_buffer_insert() {
        let mut buf = GapBuffer::<u8>::new();
        buf.push(1);
        buf.push(2);
        buf.push(3);
        buf.insert(0, 5);
        buf.insert(3, 6);
        let boxed = Box::<[u8]>::from(buf);
        assert_eq!(&[5, 1, 2, 6, 3], boxed.as_ref());
    }


    #[test]
    fn gap_buffer_delete() {
        let mut buf = GapBuffer::<u8>::new();
        buf.push(1);
        buf.push(2);
        buf.push(3);
        buf.delete(0);
        buf.delete(1);
        let boxed = Box::<[u8]>::from(buf);
        assert_eq!(&[2], boxed.as_ref());
    }

    #[test]
    fn gap_buffer_grow() {
        let mut buf = GapBuffer::<u8>::new();
        buf.gap_ensure_size(32);
        assert_eq!(buf.gap_len, 32);
        assert_eq!(buf.buffer_capacity, 32);
        buf.gap_ensure_size(64);
        assert_eq!(buf.gap_len, 64);
        assert_eq!(buf.buffer_capacity, 64);
        buf.gap_ensure_size(65);
        assert_eq!(buf.gap_len, 128);
        assert_eq!(buf.buffer_capacity, 128);
    }

    #[test]
    fn gap_buffer_drop_test() {
        let last = Rc::new(0);
        let weak = Rc::downgrade(&last);
        {
            let mut buf = GapBuffer::new();
            buf.push(last);
        };
        assert!(weak.upgrade().is_none());
    }
}