"""跨应用与适配器边界共享的领域错误。"""


class ConcurrentModificationError(RuntimeError):
    """源会话在加载后发生变化；不得用旧快照覆盖新内容。"""
