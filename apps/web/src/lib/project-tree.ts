/**
 * Build a hierarchical tree from the flat lists the backend returns.
 *
 * Backend gives us:
 *  - `items`: leaves keyed by slash-separated path (e.g. "pid_loops/temp")
 *  - `folders`: every directory path under the section root, including
 *               empty ones (so a freshly-created folder still renders)
 *
 * We produce a recursive `TreeNode` that ProjectTree can render directly.
 * Nodes are sorted: folders first, then items, both alphabetically.
 */

export type FolderNode<T> = {
  kind: "folder"
  /** Display label — just the leaf segment of the folder path. */
  name: string
  /** Full slash-separated path from the section root (e.g. "actuators/valves"). */
  path: string
  children: TreeNode<T>[]
}

export type ItemNode<T> = {
  kind: "item"
  /** Display label — just the leaf segment of the full name. */
  name: string
  /** Full path used as the API identifier. */
  path: string
  item: T
}

export type TreeNode<T> = FolderNode<T> | ItemNode<T>

/**
 * Group items + folders into a tree.
 *
 * - `getPath(item)` returns the slash-separated location of the item
 *   (e.g. "pid_loops/temp_pid"). This is the on-disk identifier
 *   (file path without extension for POU files; the `.name` field for
 *   Devices and Edges).
 * - `folders` are folder paths (e.g. ["pid_loops", "pid_loops/inner"]).
 *
 * Both lists may overlap; we dedup so an empty folder still renders
 * even when no items live inside it yet.
 */
export function buildTree<T>(
  items: T[],
  folders: string[],
  getPath: (item: T) => string,
): TreeNode<T>[] {
  // Root node carrying the section's children.
  const root: FolderNode<T> = {
    kind: "folder",
    name: "",
    path: "",
    children: [],
  }

  // Walk segments to find/create the folder node where `parts[0..parts.length-1]`
  // lives, returning that node.
  const folderAt = (parts: string[]): FolderNode<T> => {
    let node = root
    let acc = ""
    for (const segment of parts) {
      acc = acc ? `${acc}/${segment}` : segment
      let next = node.children.find(
        (c): c is FolderNode<T> => c.kind === "folder" && c.name === segment,
      )
      if (!next) {
        next = { kind: "folder", name: segment, path: acc, children: [] }
        node.children.push(next)
      }
      node = next
    }
    return node
  }

  // Pre-create every declared folder (so empty ones still appear).
  for (const folderPath of folders) {
    folderAt(folderPath.split("/"))
  }

  // Place each item under the folder its path parts imply.
  for (const item of items) {
    const fullPath = getPath(item)
    const parts = fullPath.split("/")
    const leaf = parts.pop()!
    const parent = folderAt(parts)
    parent.children.push({
      kind: "item",
      name: leaf,
      path: fullPath,
      item,
    })
  }

  // Sort: folders before items, alphabetical within each kind.
  const sortNode = (n: FolderNode<T>) => {
    n.children.sort((a, b) => {
      if (a.kind !== b.kind) return a.kind === "folder" ? -1 : 1
      return a.name.localeCompare(b.name)
    })
    for (const child of n.children) {
      if (child.kind === "folder") sortNode(child)
    }
  }
  sortNode(root)

  return root.children
}
