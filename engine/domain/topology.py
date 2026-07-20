"""会话树的纯领域规则。"""


def session_roots(rows: list[dict]) -> list[dict]:
    nodes = {}
    for source in rows:
        node = dict(source)
        node["children"] = []
        node["own_count"] = source.get("own_count", source.get("count", 0))
        node["own_size"] = source.get("own_size", source.get("size", 0))
        node["own_updated"] = source.get("own_updated", source.get("updated", 0))
        nodes[node["id"]] = node

    cyclic = set()
    for node in nodes.values():
        cursor, path = node, []
        while cursor is not None and cursor["id"] not in path:
            path.append(cursor["id"])
            cursor = nodes.get(cursor.get("parent_id"))
        if cursor is not None:
            cyclic.update(path[path.index(cursor["id"]):])

    roots = []
    for node in nodes.values():
        parent = None if node["id"] in cyclic else nodes.get(node.get("parent_id"))
        if parent is not None and parent is not node:
            parent["children"].append(node)
        else:
            node["parent_id"] = None
            roots.append(node)

    visiting = set()

    def summarize(node, root_id):
        if node["id"] in visiting:
            node["children"] = []
        visiting.add(node["id"])
        node["root_id"] = root_id
        for child in node["children"]:
            summarize(child, root_id)
        visiting.discard(node["id"])
        node["children"].sort(key=lambda child: child.get("updated", 0), reverse=True)
        node["child_count"] = len(node["children"])
        node["tree_count"] = 1 + sum(child["tree_count"] for child in node["children"])
        node["count"] = node["own_count"] + sum(child["count"] for child in node["children"])
        node["size"] = node["own_size"] + sum(child["size"] for child in node["children"])
        node["updated"] = max([node["own_updated"], *(child["updated"] for child in node["children"])])

    for root in roots:
        summarize(root, root["id"])
    roots.sort(key=lambda node: node["updated"], reverse=True)
    return roots
