use std::{alloc, fmt, mem, ptr, slice};
use std::cmp::max;
use std::fmt::{Formatter, Write};
use std::ops::Add;

pub struct GapBuffer<T> {
    buffer: ptr::NonNull<T>,
    buffer_capacity: usize,
    gap_start: usize,
    gap_len: usize,
}

impl<T> GapBuffer<T> {
    const MIN_NON_ZERO_CAP: usize = if mem::size_of::<T>() == 1 {
        8
    } else if mem::size_of::<T>() <= 1024 {
        4
    } else {
        1
    };

    pub fn new() -> Self {
        Self {
            buffer: ptr::NonNull::dangling(),
            buffer_capacity: 0,
            gap_start: 0,
            gap_len: 0,
        }
    }

    pub fn push(&mut self, value: T) {
        self.insert(self.buffer_size(), value)
    }

    pub fn insert(&mut self, i: usize, value: T) {
        if i > self.buffer_size() {
            panic!("Index out of bound for {:?} of buffer's size {:?}", i, self.buffer_size())
        }
        self.gap_move_to(i);
        self.gap_ensure_size(1);
        unsafe {
            *self.buffer.as_ptr().add(self.gap_start) = value;
        }
        self.gap_start += 1;
        self.gap_len -= 1;
    }

    pub fn delete(&mut self, i: usize) {
        if self.buffer_size() == 0 || i > self.buffer_size() {
            panic!("Index out of bound for {:?} of buffer's size {:?}", i, self.buffer_size())
        }
        self.gap_move_to(i);
        self.gap_len += 1;
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
            let new_gap_len = new_capacity - self.buffer_size();
            let new_layout = alloc::Layout::array::<T>(new_capacity).unwrap();
            let new_buffer = unsafe { alloc::alloc(new_layout) } as *mut T;
            let buffer = self.buffer.as_ptr();
            unsafe {
                let src = buffer;
                let dst = new_buffer;
                let count = self.gap_start;
                ptr::copy_nonoverlapping::<T>(src, dst, count);

                let src = buffer.add(self.gap_start).add(self.gap_len);
                let dst = new_buffer.add(self.gap_start).add(new_gap_len);
                let count = self.buffer_capacity - self.gap_len - self.gap_start;
                ptr::copy_nonoverlapping::<T>(src, dst, count);
            }
            if let Some(layout) = self.buffer_layout() {
                unsafe { alloc::dealloc(buffer as *mut u8, layout) }
            }
            self.buffer = ptr::NonNull::new(new_buffer).unwrap();
            self.buffer_capacity = new_capacity;
            self.gap_len = new_gap_len;
        }
    }

    fn buffer_size(&self) -> usize {
        self.buffer_capacity - self.gap_len
    }

    fn buffer_layout(&self) -> Option<alloc::Layout> {
        if self.buffer_capacity == 0 {
            None
        } else {
            Some(alloc::Layout::array::<T>(self.buffer_capacity).unwrap())
        }
    }
}

impl<T: Clone> From<GapBuffer<T>> for Box<[T]> {
    fn from(value: GapBuffer<T>) -> Self {
        let ptr = value.buffer.as_ptr();
        let first = unsafe { slice::from_raw_parts(ptr, value.gap_start) };
        let second = unsafe { slice::from_raw_parts(ptr.add(value.gap_start).add(value.gap_len), value.buffer_capacity - value.gap_len - value.gap_start) };
        [first, second].concat().into_boxed_slice()
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
    use crate::GapBuffer;

    #[test]
    fn gap_buffer_with_capacity_8() {
        let mut buf = GapBuffer::<u8>::new();
        buf.gap_ensure_size(1);
        assert_eq!(buf.gap_len, GapBuffer::<u8>::MIN_NON_ZERO_CAP)
    }

    #[test]
    fn gap_buffer_with_capacity_4() {
        let mut buf = GapBuffer::<[u8; 1024]>::new();
        buf.gap_ensure_size(1);
        assert_eq!(buf.gap_len, GapBuffer::<[u8; 1024]>::MIN_NON_ZERO_CAP)
    }

    #[test]
    fn gap_buffer_with_capacity_1() {
        let mut buf = GapBuffer::<[u8; 1025]>::new();
        buf.gap_ensure_size(1);
        assert_eq!(buf.gap_len, GapBuffer::<[u8; 1025]>::MIN_NON_ZERO_CAP)
    }

    #[test]
    fn gap_buffer_push() {
        let mut buf = GapBuffer::<u8>::new();
        buf.push(1);
        buf.push(2);
        buf.push(3);
        let boxed = Box::<[u8]>::from(buf);
        assert_eq!(&[1, 2, 3], boxed.as_ref())
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
        assert_eq!(&[5, 1, 2, 6, 3], boxed.as_ref())
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
        assert_eq!(&[2], boxed.as_ref())
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
}