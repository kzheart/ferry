"""BLAKE3 的纯 Python 实现，供 Grok 搜索索引计算内容摘要。

从 grok/writer.py 原样迁出，算法逐字节未改；不引入原生 blake3 依赖是为了
避免 PyInstaller onefile 打包的跨平台成本。
"""
from __future__ import annotations

import struct


_IV = (
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
)
_MSG_PERMUTATION = (2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8)
_CHUNK_START, _CHUNK_END, _PARENT, _ROOT = 1, 2, 4, 8


def _rotate_right(value, shift):
    return ((value >> shift) | (value << (32 - shift))) & 0xFFFFFFFF


def _mix(state, a, b, c, d, x, y):
    state[a] = (state[a] + state[b] + x) & 0xFFFFFFFF
    state[d] = _rotate_right(state[d] ^ state[a], 16)
    state[c] = (state[c] + state[d]) & 0xFFFFFFFF
    state[b] = _rotate_right(state[b] ^ state[c], 12)
    state[a] = (state[a] + state[b] + y) & 0xFFFFFFFF
    state[d] = _rotate_right(state[d] ^ state[a], 8)
    state[c] = (state[c] + state[d]) & 0xFFFFFFFF
    state[b] = _rotate_right(state[b] ^ state[c], 7)


def _round(state, words):
    _mix(state, 0, 4, 8, 12, words[0], words[1])
    _mix(state, 1, 5, 9, 13, words[2], words[3])
    _mix(state, 2, 6, 10, 14, words[4], words[5])
    _mix(state, 3, 7, 11, 15, words[6], words[7])
    _mix(state, 0, 5, 10, 15, words[8], words[9])
    _mix(state, 1, 6, 11, 12, words[10], words[11])
    _mix(state, 2, 7, 8, 13, words[12], words[13])
    _mix(state, 3, 4, 9, 14, words[14], words[15])


def _compress(chaining_value, block_words, counter, block_len, flags):
    state = list(chaining_value) + list(_IV[:4]) + [
        counter & 0xFFFFFFFF, counter >> 32, block_len, flags,
    ]
    words = list(block_words)
    for _ in range(7):
        _round(state, words)
        words = [words[index] for index in _MSG_PERMUTATION]
    return tuple(
        [state[index] ^ state[index + 8] for index in range(8)]
        + [state[index + 8] ^ chaining_value[index] for index in range(8)]
    )


class _Output:
    def __init__(self, chaining_value, words, counter, block_len, flags):
        self.input_chaining_value = chaining_value
        self.words = words
        self.counter = counter
        self.block_len = block_len
        self.flags = flags

    def chaining_value(self):
        return _compress(
            self.input_chaining_value, self.words, self.counter,
            self.block_len, self.flags,
        )[:8]

    def root_bytes(self):
        words = _compress(
            self.input_chaining_value, self.words, 0,
            self.block_len, self.flags | _ROOT,
        )
        return struct.pack("<16I", *words)[:32]


def _block_words(block):
    return struct.unpack("<16I", block.ljust(64, b"\0"))


def _chunk_output(chunk, counter):
    chaining_value = _IV
    blocks = [chunk[index:index + 64] for index in range(0, len(chunk), 64)]
    if not blocks:
        blocks = [b""]
    for index, block in enumerate(blocks[:-1]):
        flags = _CHUNK_START if index == 0 else 0
        chaining_value = _compress(
            chaining_value, _block_words(block), counter, len(block), flags,
        )[:8]
    last_index = len(blocks) - 1
    flags = _CHUNK_END | (_CHUNK_START if last_index == 0 else 0)
    return _Output(
        chaining_value, _block_words(blocks[-1]), counter,
        len(blocks[-1]), flags,
    )


def _parent_output(left, right):
    return _Output(_IV, tuple(left) + tuple(right), 0, 64, _PARENT)


def blake3_hex(value):
    chunks = [value[index:index + 1024]
              for index in range(0, len(value), 1024)] or [b""]
    stack = []
    for counter, chunk in enumerate(chunks[:-1]):
        chaining_value = _chunk_output(chunk, counter).chaining_value()
        total_chunks = counter + 1
        while total_chunks & 1 == 0:
            chaining_value = _parent_output(
                stack.pop(), chaining_value,
            ).chaining_value()
            total_chunks >>= 1
        stack.append(chaining_value)
    output = _chunk_output(chunks[-1], len(chunks) - 1)
    for left in reversed(stack):
        output = _parent_output(left, output.chaining_value())
    return output.root_bytes().hex()
