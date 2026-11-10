// Server-side guard: Figma writes only go to the file the AI checked (get_document_info / design_map),
// and only while that file is still the one open. Plugin-side guards (delete, approval, review) live in code.ts.

// Tools that change the Figma document. Everything else reads, or works outside Figma.
export const FIGMA_WRITES = new Set([
  "create_frame", "create_rectangle", "create_ellipse", "create_text", "create_instance", "create_component",
  "combine_as_variants", "create_section", "add_component_property", "create_paint_style", "create_text_style",
  "duplicate_node", "group_nodes", "ungroup_node", "set_text", "set_text_props", "set_fill", "set_stroke",
  "set_corner_radius", "set_effects", "set_props", "set_auto_layout", "set_sizing", "move_resize", "move_to_parent",
  "rename_node", "delete_node", "set_image_fill", "copy_fills", "create_polygon", "create_star", "create_line",
  "create_from_svg", "boolean_shapes", "flatten", "outline_stroke", "vector_edit", "bend_shape", "use_as_mask", "remove_mask", "add_interaction",
  "remove_interactions", "set_flow_start", "set_scroll", "apply_style", "bind_variable", "create_variable_collection",
  "create_variables", "set_variable_value", "set_instance_properties",
  "create_sticky", "create_shape", "create_connector", "figjam_draw", "motion_animate",
]);

/**
 * Wraps send(command, args): remembers the file a check returned, and refuses writes when no file was
 * checked yet or another file is open now. The open file is re-read at most every `ttlMs` (batches), and
 * always after `epoch()` changed (a window closed, or use_file switched).
 */
export function createFileGuard(send, { ttlMs = 2000, now = () => Date.now(), epoch = () => 0 } = {}) {
  let checked = null;
  let seen = { name: null, at: -Infinity, epoch: 0 };

  const openFile = async () => {
    const e = epoch();
    if (now() - seen.at < ttlMs && seen.epoch === e) return seen.name;
    const { fileName } = await send("file_name", {});
    seen = { name: fileName, at: now(), epoch: e };
    return fileName;
  };

  return async function guardedSend(command, args, ...rest) {
    if (FIGMA_WRITES.has(command)) {
      if (!checked) {
        throw new Error("Check which Figma file is open before changing it: call get_document_info, and tell the user which file you will change.");
      }
      const open = await openFile();
      if (open !== checked) {
        const before = checked;
        checked = null; // a new check is required, not just a retry
        throw new Error(`The open Figma file changed from "${before}" to "${open}" since you checked. Confirm with the user which file to change, then call get_document_info again.`);
      }
    }
    const before = epoch();
    const result = await send(command, args, ...rest);
    if (command === "get_document_info" && result?.fileName) checked = result.fileName;
    if (command === "design_map" && result?.file) checked = result.file;
    if (checked) seen = { name: checked, at: now(), epoch: before };
    return result;
  };
}
