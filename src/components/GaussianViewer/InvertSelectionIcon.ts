import { createLucideIcon } from "lucide-react";

// Solid and dashed regions exchange places, rather than mirroring the model.
export const InvertSelectionIcon = createLucideIcon("SelectionInvert", [
  ["rect", { x: "3", y: "3", width: "7", height: "7", rx: "1", fill: "currentColor", fillOpacity: "0.16", key: "selected" }],
  ["rect", { x: "14", y: "14", width: "7", height: "7", rx: "1", strokeDasharray: "2 2", key: "unselected" }],
  ["path", { d: "M14 3h3a3 3 0 0 1 3 3v4", key: "to-unselected" }],
  ["path", { d: "m17 7 3 3 3-3", key: "down-arrow" }],
  ["path", { d: "M10 21H7a3 3 0 0 1-3-3v-4", key: "to-selected" }],
  ["path", { d: "m1 17 3-3 3 3", key: "up-arrow" }],
]);
