<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { X } from "lucide-svelte";
  import { apiClient } from "../../lib/api";
  import type {
    MemoryChildEntry,
    MemoryNodeDetail,
    MemoryNodeKind,
    MemorySidebarItem,
    MemorySidebarSection
  } from "../../lib/types";
  import {
    formatMemoryTimestamp,
    memoryKindLabel,
    routeLabel,
    routeSegment
  } from "./memory";

  interface GraphNode {
    renderKey: string;
    id: string;
    key: string;
    kind: MemoryNodeKind;
    label: string;
    subtitle: string;
    sectionKey: string;
    sectionTitle: string;
    uri: string | null;
    x: number;
    y: number;
    width: number;
    height: number;
    depth: number;
    detail: MemoryNodeDetail | null;
  }

  interface GraphEdgeRecord {
    renderKey: string;
    id: string;
    sourceId: string;
    targetId: string;
    relationKind: string;
    visibility: string;
    priority: number;
    triggerText: string | null;
    kind: "relation" | "related";
    isTreePath: boolean;
  }

  interface GraphPopoverState {
    renderKey: string;
    x: number;
    y: number;
  }

  let {
    memorySections,
    onClose
  }: {
    memorySections: MemorySidebarSection[];
    onClose: () => void;
  } = $props();

  const nodeWidth = 216;
  const nodeHeight = 88;
  const treeColumnWidth = 304;
  const treeRowHeight = 126;
  const graphPaddingX = 88;
  const graphPaddingY = 120;

  function memoryGraphDebug(message: string, payload?: Record<string, unknown>) {
    console.log("[memory-graph][Modal]", message, payload ?? {});
  }

  let graphLoading = $state(false);
  let graphError = $state<string | null>(null);
  let graphNotice = $state<string | null>(null);
  let graphNodes = $state<GraphNode[]>([]);
  let graphEdges = $state<GraphEdgeRecord[]>([]);
  let loadedSignature = $state("");
  let requestToken = $state(0);
  let activePopover = $state<GraphPopoverState | null>(null);
  let graphViewport: HTMLDivElement | null = $state(null);
  let zoom = $state(1);
  let isPanning = $state(false);
  let panPointerId = $state<number | null>(null);
  let panStartX = $state(0);
  let panStartY = $state(0);
  let panScrollLeft = $state(0);
  let panScrollTop = $state(0);

  const contentSections = $derived(
    memorySections.filter((section) => section.key !== "reviews" && section.items.length > 0)
  );

  const graphSignature = $derived(
    contentSections
      .map((section) => `${section.key}:${section.items.map((item) => item.node_id).join(",")}`)
      .join("|")
  );

  const canvasWidth = $derived.by(() => {
    const farthestNode = Math.max(...graphNodes.map((node) => node.x + node.width), graphPaddingX + nodeWidth);
    return Math.max(1180, farthestNode + graphPaddingX + 160);
  });

  const canvasHeight = $derived.by(() => {
    const lowestNode = Math.max(...graphNodes.map((node) => node.y + node.height), graphPaddingY + nodeHeight);
    return Math.max(720, lowestNode + graphPaddingY + 96);
  });

  const depthColumns = $derived.by(() => {
    const maxDepth = Math.max(...graphNodes.map((node) => node.depth), 0);
    return Array.from({ length: maxDepth + 1 }, (_, depth) => ({
      depth,
      label: depth === 0 ? "根层" : `第 ${depth + 1} 层`,
      x: graphPaddingX + depth * treeColumnWidth
    }));
  });

  const scaledCanvasWidth = $derived(Math.max(960, Math.round(canvasWidth * zoom)));
  const scaledCanvasHeight = $derived(Math.max(620, Math.round(canvasHeight * zoom)));

  const activeNode = $derived.by(
    () => activePopover ? graphNodes.find((node) => node.renderKey === activePopover.renderKey) ?? null : null
  );

  const highlightState = $derived.by(() => {
    if (!activeNode) {
      return {
        nodeIds: new Set<string>(),
        edgeIds: new Set<string>()
      };
    }

    const visitedNodes = new Set<string>([activeNode.id]);
    const visitedEdges = new Set<string>();
    const queue: Array<{ nodeId: string; depth: number }> = [{ nodeId: activeNode.id, depth: 0 }];
    const maxDepth = 2;

    while (queue.length > 0) {
      const current = queue.shift();
      if (!current) {
        continue;
      }

      for (const edge of graphEdges) {
        if (edge.sourceId !== current.nodeId && edge.targetId !== current.nodeId) {
          continue;
        }

        visitedEdges.add(edge.renderKey);
        const neighborId = edge.sourceId === current.nodeId ? edge.targetId : edge.sourceId;

        if (!visitedNodes.has(neighborId)) {
          visitedNodes.add(neighborId);
          if (current.depth < maxDepth) {
            queue.push({ nodeId: neighborId, depth: current.depth + 1 });
          }
        }
      }
    }

    return {
      nodeIds: visitedNodes,
      edgeIds: visitedEdges
    };
  });

  const popoverStyle = $derived.by(() => {
    if (!activePopover || !activeNode) {
      return "";
    }

    const width = 360;
    const height = 420;
    const left = Math.max(
      20,
      Math.min(activePopover.x + 18, scaledCanvasWidth - width - 20)
    );
    const top = Math.max(
      20,
      Math.min(activePopover.y - 28, scaledCanvasHeight - height - 20)
    );

    return `left:${left}px; top:${top}px;`;
  });

  $effect(() => {
    const signature = graphSignature;
    if (!signature || signature === loadedSignature) {
      return;
    }
    void loadGraph(signature, contentSections);
  });

  $effect(() => {
    if (!activePopover) {
      return;
    }
    const node = graphNodes.find((entry) => entry.renderKey === activePopover.renderKey);
    if (!node) {
      return;
    }

    const nextX = (node.x + node.width * 0.66) * zoom;
    const nextY = (node.y + node.height * 0.54) * zoom;

    if (
      Math.abs(activePopover.x - nextX) <= 0.5 &&
      Math.abs(activePopover.y - nextY) <= 0.5
    ) {
      return;
    }

    activePopover = {
      renderKey: node.renderKey,
      x: nextX,
      y: nextY
    };
  });

  function sectionColor(kind: MemoryNodeKind) {
    switch (kind) {
      case "boot":
        return { fill: "#e8f1ff", stroke: "#6b8ee6" };
      case "identity":
      case "user_profile":
        return { fill: "#fff0e4", stroke: "#d78b3f" };
      case "directive":
      case "procedure":
        return { fill: "#eef8ea", stroke: "#5f9b63" };
      case "episode":
        return { fill: "#f4f0ff", stroke: "#8b73cf" };
      case "value":
      case "curated":
        return { fill: "#fff7da", stroke: "#b28c2d" };
      default:
        return { fill: "#f3f5f8", stroke: "#8390a2" };
    }
  }

  function summarizeNode(item: MemorySidebarItem, detail: MemoryNodeDetail | null) {
    if (item.subtitle) {
      return item.subtitle;
    }
    if (detail?.selected_uri) {
      return detail.selected_uri;
    }
    if (detail?.primary_route) {
      return routeLabel(detail.primary_route);
    }
    return memoryKindLabel(item.kind);
  }

  function buildBaseNodes(sections: MemorySidebarSection[], details: Map<string, MemoryNodeDetail | null>) {
    const groupedItems = new Map<
      string,
      {
        item: MemorySidebarItem;
        sectionKey: string;
        sectionTitles: Set<string>;
        sortIndex: number;
      }
    >();

    sections.forEach((section, sectionIndex) => {
      section.items.forEach((item, itemIndex) => {
        const existing = groupedItems.get(item.node_id);
        if (existing) {
          existing.sectionTitles.add(section.title);
          return;
        }

        groupedItems.set(item.node_id, {
          item,
          sectionKey: section.key,
          sectionTitles: new Set([section.title]),
          sortIndex: sectionIndex * 1000 + itemIndex
        });
      });
    });

    return [...groupedItems.values()]
      .sort((left, right) => left.sortIndex - right.sortIndex)
      .map((entry, index) => {
        const detail = details.get(entry.item.node_id) ?? null;
        return {
          renderKey: `${entry.item.node_id}:${index}`,
          id: entry.item.node_id,
          key: entry.item.uri ?? entry.item.node_id,
          kind: entry.item.kind,
          label: routeSegment(entry.item.uri) ?? entry.item.title,
          subtitle: summarizeNode(entry.item, detail),
          sectionKey: entry.sectionKey,
          sectionTitle: [...entry.sectionTitles].join(" / "),
          uri: entry.item.uri,
          x: graphPaddingX,
          y: graphPaddingY,
          width: nodeWidth,
          height: nodeHeight,
          depth: 0,
          detail
        } satisfies GraphNode;
      });
  }

  function buildEdges(nodes: GraphNode[], details: Map<string, MemoryNodeDetail | null>) {
    const nodeIds = new Set(nodes.map((node) => node.id));
    const dedupe = new Set<string>();
    const edges: GraphEdgeRecord[] = [];

    for (const node of nodes) {
      const detail = details.get(node.id);
      if (!detail) {
        continue;
      }

      for (const edge of detail.edges) {
        if (!edge.parent_node_id || !nodeIds.has(edge.parent_node_id) || !nodeIds.has(edge.child_node_id)) {
          continue;
        }
        const key = `${edge.parent_node_id}:${edge.child_node_id}:${edge.relation_kind}`;
        if (dedupe.has(key)) {
          continue;
        }
        dedupe.add(key);
        edges.push({
          renderKey: `${edge.parent_node_id}:${edge.child_node_id}:${edge.relation_kind}:${edge.id}`,
          id: edge.id,
          sourceId: edge.parent_node_id,
          targetId: edge.child_node_id,
          relationKind: edge.relation_kind,
          visibility: edge.visibility,
          priority: edge.priority,
          triggerText: edge.trigger_text,
          kind: "relation",
          isTreePath: false
        });
      }

      for (const related of detail.related_nodes) {
        if (!nodeIds.has(related.node_id)) {
          continue;
        }
        const key = `${node.id}:${related.node_id}:related`;
        const reverseKey = `${related.node_id}:${node.id}:related`;
        if (dedupe.has(key) || dedupe.has(reverseKey)) {
          continue;
        }
        dedupe.add(key);
        edges.push({
          renderKey: `${key}:${edges.length}`,
          id: key,
          sourceId: node.id,
          targetId: related.node_id,
          relationKind: "related",
          visibility: "linked",
          priority: related.priority,
          triggerText: related.trigger_text,
          kind: "related",
          isTreePath: false
        });
      }
    }

    return edges;
  }

  function layoutTree(nodes: GraphNode[], edges: GraphEdgeRecord[]) {
    const nodeById = new Map(nodes.map((node) => [node.id, node]));
    const orderById = new Map(nodes.map((node, index) => [node.id, index]));
    const parentById = new Map<string, string>();
    const treeEdgeKeys = new Set<string>();
    const childrenById = new Map<string, string[]>(
      nodes.map((node) => [node.id, []])
    );

    function edgeSort(left: GraphEdgeRecord, right: GraphEdgeRecord) {
      if (left.kind !== right.kind) {
        return left.kind === "relation" ? -1 : 1;
      }
      if (right.priority !== left.priority) {
        return right.priority - left.priority;
      }
      if (left.sourceId !== right.sourceId) {
        return (orderById.get(left.sourceId) ?? 0) - (orderById.get(right.sourceId) ?? 0);
      }
      return (orderById.get(left.targetId) ?? 0) - (orderById.get(right.targetId) ?? 0);
    }

    function wouldCreateCycle(sourceId: string, targetId: string) {
      let current = sourceId;
      while (current) {
        if (current === targetId) {
          return true;
        }
        current = parentById.get(current) ?? "";
      }
      return false;
    }

    const candidateEdges = [...edges].sort(edgeSort);
    for (const edge of candidateEdges) {
      if (edge.sourceId === edge.targetId) {
        continue;
      }
      if (!nodeById.has(edge.sourceId) || !nodeById.has(edge.targetId)) {
        continue;
      }
      if (parentById.has(edge.targetId)) {
        continue;
      }
      if (wouldCreateCycle(edge.sourceId, edge.targetId)) {
        continue;
      }

      parentById.set(edge.targetId, edge.sourceId);
      childrenById.get(edge.sourceId)?.push(edge.targetId);
      treeEdgeKeys.add(edge.renderKey);
    }

    for (const childIds of childrenById.values()) {
      childIds.sort((left, right) => (orderById.get(left) ?? 0) - (orderById.get(right) ?? 0));
    }

    const centerById = new Map<string, number>();
    const depthById = new Map<string, number>();
    let leafCursor = 0;

    function placeNode(nodeId: string, depth: number, trail = new Set<string>()) {
      if (centerById.has(nodeId)) {
        return centerById.get(nodeId) ?? 0;
      }

      if (trail.has(nodeId)) {
        const fallbackCenter = leafCursor;
        leafCursor += 1;
        centerById.set(nodeId, fallbackCenter);
        depthById.set(nodeId, depth);
        return fallbackCenter;
      }

      const nextTrail = new Set(trail);
      nextTrail.add(nodeId);
      depthById.set(nodeId, depth);

      const children = (childrenById.get(nodeId) ?? []).filter((childId) => childId !== nodeId);
      if (children.length === 0) {
        const center = leafCursor;
        leafCursor += 1;
        centerById.set(nodeId, center);
        return center;
      }

      const childCenters = children.map((childId) => placeNode(childId, depth + 1, nextTrail));
      const center = (childCenters[0] + childCenters[childCenters.length - 1]) / 2;
      centerById.set(nodeId, center);
      return center;
    }

    const rootIds = nodes
      .map((node) => node.id)
      .filter((nodeId) => !parentById.has(nodeId));

    rootIds.forEach((rootId, index) => {
      placeNode(rootId, 0);
      if (index < rootIds.length - 1) {
        leafCursor += 0.7;
      }
    });

    nodes.forEach((node) => {
      if (!centerById.has(node.id)) {
        if (leafCursor > 0) {
          leafCursor += 0.7;
        }
        placeNode(node.id, 0);
      }
    });

    const laidOutNodes = nodes.map((node) => {
      const depth = depthById.get(node.id) ?? 0;
      const center = centerById.get(node.id) ?? 0;
      return {
        ...node,
        depth,
        x: graphPaddingX + depth * treeColumnWidth,
        y: graphPaddingY + center * treeRowHeight
      };
    });

    const laidOutEdges = edges.map((edge) => ({
      ...edge,
      isTreePath: treeEdgeKeys.has(edge.renderKey)
    }));

    return {
      nodes: laidOutNodes,
      edges: laidOutEdges
    };
  }

  function buildChildNodes(
    baseNodes: GraphNode[],
    childrenMap: Map<string, MemoryChildEntry[]>
  ): GraphNode[] {
    const nodeIds = new Set(baseNodes.map((n) => n.id));
    const childNodes: GraphNode[] = [];
    let sortIndex = baseNodes.length;

    for (const children of childrenMap.values()) {
      for (const child of children) {
        if (nodeIds.has(child.node_id)) {
          continue;
        }
        nodeIds.add(child.node_id);
        childNodes.push({
          renderKey: `${child.node_id}:child:${sortIndex}`,
          id: child.node_id,
          key: child.uri ?? child.node_id,
          kind: child.kind,
          label: routeSegment(child.uri) ?? child.title,
          subtitle: child.title,
          sectionKey: "children",
          sectionTitle: "Children",
          uri: child.uri,
          x: graphPaddingX,
          y: graphPaddingY,
          width: nodeWidth,
          height: nodeHeight,
          depth: 0,
          detail: null
        });
        sortIndex += 1;
      }
    }

    return childNodes;
  }

  function buildChildEdges(
    baseNodes: GraphNode[],
    childNodes: GraphNode[],
    childrenMap: Map<string, MemoryChildEntry[]>
  ): GraphEdgeRecord[] {
    const baseNodeIds = new Set(baseNodes.map((n) => n.id));
    const childNodeIds = new Set(childNodes.map((n) => n.id));
    const dedupe = new Set<string>();
    const edges: GraphEdgeRecord[] = [];

    for (const [parentId, children] of childrenMap.entries()) {
      if (!baseNodeIds.has(parentId)) {
        continue;
      }

      for (const child of children) {
        if (!childNodeIds.has(child.node_id)) {
          continue;
        }

        const key = `${parentId}:${child.node_id}:contains`;
        if (dedupe.has(key)) {
          continue;
        }
        dedupe.add(key);
        edges.push({
          renderKey: `${key}:${edges.length}`,
          id: key,
          sourceId: parentId,
          targetId: child.node_id,
          relationKind: "contains",
          visibility: "linked",
          priority: child.priority,
          triggerText: child.disclosure,
          kind: "relation",
          isTreePath: true
        });
      }
    }

    return edges;
  }

  function refreshGraph(
    sections: MemorySidebarSection[],
    details: Map<string, MemoryNodeDetail | null>,
    childrenMap: Map<string, MemoryChildEntry[]>
  ) {
    const baseNodes = buildBaseNodes(sections, details);
    const childNodes = buildChildNodes(baseNodes, childrenMap);
    const allNodes = [...baseNodes, ...childNodes];
    const baseEdges = buildEdges(allNodes, details);
    const childEdges = buildChildEdges(baseNodes, childNodes, childrenMap);
    const allEdges = [...baseEdges, ...childEdges];

    const edgeByKey = new Map<string, GraphEdgeRecord>();
    for (const edge of allEdges) {
      const key = `${edge.sourceId}:${edge.targetId}:${edge.relationKind}`;
      if (!edgeByKey.has(key)) {
        edgeByKey.set(key, edge);
      }
    }
    const dedupedEdges = [...edgeByKey.values()];

    const layout = layoutTree(allNodes, dedupedEdges);

    graphNodes = layout.nodes;
    graphEdges = layout.edges;

    const duplicateEdgeIds = [...new Set(
      layout.edges
        .map((edge) => edge.id)
        .filter((edgeId, index, allIds) => allIds.indexOf(edgeId) !== index)
    )];

    if (duplicateEdgeIds.length > 0) {
      memoryGraphDebug("duplicate logical ids detected", {
        duplicateEdgeIds
      });
    }
  }

  function edgePath(edge: GraphEdgeRecord) {
    const source = graphNodes.find((node) => node.id === edge.sourceId);
    const target = graphNodes.find((node) => node.id === edge.targetId);
    if (!source || !target) {
      return "";
    }

    const sourceX = source.x + source.width;
    const sourceY = source.y + source.height / 2;
    const targetX = target.x;
    const targetY = target.y + target.height / 2;
    const delta = Math.max(72, Math.abs(targetX - sourceX));
    const sourceControlX = sourceX + Math.min(118, delta * 0.45);
    const targetControlX = targetX - Math.min(92, delta * 0.3);

    if (edge.isTreePath) {
      return `M ${sourceX} ${sourceY} C ${sourceControlX} ${sourceY}, ${targetControlX} ${targetY}, ${targetX} ${targetY}`;
    }

    const crossControl = Math.max(80, delta * 0.32);
    return `M ${sourceX} ${sourceY} C ${sourceX + crossControl} ${sourceY}, ${targetX - crossControl} ${targetY}, ${targetX} ${targetY}`;
  }

  function clampZoom(value: number) {
    return Math.min(2.2, Math.max(0.55, Number(value.toFixed(2))));
  }

  function nodeAnchorPosition(node: GraphNode) {
    return {
      x: (node.x + node.width * 0.66) * zoom,
      y: (node.y + node.height * 0.54) * zoom
    };
  }

  function nodeMeta(node: GraphNode) {
    if (node.detail?.selected_uri) {
      return node.detail.selected_uri;
    }
    if (node.uri) {
      return node.uri;
    }
    return `${memoryKindLabel(node.kind)} node`;
  }

  function openNodePopover(node: GraphNode, event: MouseEvent) {
    event.stopPropagation();
    if (!graphViewport) {
      const anchor = nodeAnchorPosition(node);
      activePopover = { renderKey: node.renderKey, x: anchor.x, y: anchor.y };
      return;
    }

    const rect = graphViewport.getBoundingClientRect();
    activePopover = {
      renderKey: node.renderKey,
      x: event.clientX - rect.left + graphViewport.scrollLeft,
      y: event.clientY - rect.top + graphViewport.scrollTop
    };
  }

  function openNodePopoverFromKeyboard(node: GraphNode, event: KeyboardEvent) {
    if (event.key !== "Enter" && event.key !== " ") {
      return;
    }
    event.preventDefault();
    const anchor = nodeAnchorPosition(node);
    activePopover = { renderKey: node.renderKey, x: anchor.x, y: anchor.y };
  }

  function beginPan(event: PointerEvent) {
    const target = event.target as HTMLElement | null;
    if (!graphViewport || !target) {
      return;
    }
    if (target.closest(".graph-node") || target.closest(".graph-popover") || target.closest(".graph-controls")) {
      return;
    }

    isPanning = true;
    panPointerId = event.pointerId;
    panStartX = event.clientX;
    panStartY = event.clientY;
    panScrollLeft = graphViewport.scrollLeft;
    panScrollTop = graphViewport.scrollTop;
    graphViewport.setPointerCapture(event.pointerId);
  }

  function updatePan(event: PointerEvent) {
    if (!graphViewport || !isPanning || panPointerId !== event.pointerId) {
      return;
    }

    const deltaX = event.clientX - panStartX;
    const deltaY = event.clientY - panStartY;
    graphViewport.scrollLeft = panScrollLeft - deltaX;
    graphViewport.scrollTop = panScrollTop - deltaY;
  }

  function endPan(event: PointerEvent) {
    if (!graphViewport || panPointerId !== event.pointerId) {
      return;
    }

    if (graphViewport.hasPointerCapture(event.pointerId)) {
      graphViewport.releasePointerCapture(event.pointerId);
    }
    isPanning = false;
    panPointerId = null;
  }

  function applyZoom(nextZoom: number, originX?: number, originY?: number) {
    if (!graphViewport) {
      zoom = clampZoom(nextZoom);
      return;
    }

    const rect = graphViewport.getBoundingClientRect();
    const targetZoom = clampZoom(nextZoom);
    if (targetZoom === zoom) {
      return;
    }

    const relativeX = originX ?? rect.width / 2;
    const relativeY = originY ?? rect.height / 2;
    const contentX = graphViewport.scrollLeft + relativeX;
    const contentY = graphViewport.scrollTop + relativeY;
    const worldX = contentX / zoom;
    const worldY = contentY / zoom;

    zoom = targetZoom;

    requestAnimationFrame(() => {
      if (!graphViewport) {
        return;
      }
      graphViewport.scrollLeft = worldX * targetZoom - relativeX;
      graphViewport.scrollTop = worldY * targetZoom - relativeY;
    });
  }

  function handleViewportWheel(event: WheelEvent) {
    if (!graphViewport || !(event.ctrlKey || event.metaKey)) {
      return;
    }

    event.preventDefault();
    const rect = graphViewport.getBoundingClientRect();
    applyZoom(
      zoom * (event.deltaY > 0 ? 0.9 : 1.1),
      event.clientX - rect.left,
      event.clientY - rect.top
    );
  }

  function zoomIn() {
    applyZoom(zoom + 0.15);
  }

  function zoomOut() {
    applyZoom(zoom - 0.15);
  }

  function resetView() {
    zoom = 1;
    activePopover = activeNode
      ? {
          renderKey: activeNode.renderKey,
          ...nodeAnchorPosition(activeNode)
        }
      : null;

    requestAnimationFrame(() => {
      if (!graphViewport) {
        return;
      }
      graphViewport.scrollTo({ top: 0, left: 0, behavior: "smooth" });
    });
  }

  function handleClose(event?: Event) {
    memoryGraphDebug("handleClose()", {
      eventType: event?.type ?? null,
      eventTarget:
        event?.target instanceof Element
          ? `${event.target.tagName.toLowerCase()}.${event.target.className}`
          : null,
      graphLoading,
      requestToken
    });
    event?.preventDefault();
    event?.stopPropagation();
    requestToken += 1;
    graphLoading = false;
    isPanning = false;
    panPointerId = null;
    activePopover = null;
    onClose();
  }

  async function loadNodeDetailWithTimeout(key: string, timeoutMs = 2500) {
    let timer: ReturnType<typeof setTimeout> | null = null;
    try {
      return await Promise.race([
        apiClient.getMemoryNode(key),
        new Promise<never>((_, reject) => {
          timer = setTimeout(() => reject(new Error(`Memory node request timed out: ${key}`)), timeoutMs);
        })
      ]);
    } finally {
      if (timer !== null) {
        clearTimeout(timer);
      }
    }
  }

  async function loadGraph(signature: string, sections: MemorySidebarSection[]) {
    const token = requestToken + 1;
    requestToken = token;
    graphLoading = true;
    graphError = null;
    graphNotice = null;
    activePopover = null;
    loadedSignature = signature;

    try {
      const items = sections.flatMap((section) => section.items);
      const uniqueItems = items.filter(
        (item, index, array) => array.findIndex((candidate) => candidate.node_id === item.node_id) === index
      );
      const detailMap = new Map<string, MemoryNodeDetail | null>();
      const childrenMap = new Map<string, MemoryChildEntry[]>();
      refreshGraph(sections, detailMap, childrenMap);

      const failedKeys: string[] = [];

      for (const item of uniqueItems) {
        if (requestToken !== token) {
          return;
        }

        try {
          const response = await loadNodeDetailWithTimeout(item.uri ?? item.node_id);
          if (requestToken !== token) {
            return;
          }
          detailMap.set(item.node_id, response.detail);
        } catch (error) {
          if (requestToken !== token) {
            return;
          }
          failedKeys.push(routeSegment(item.uri) ?? item.title);
          detailMap.set(item.node_id, null);
        }

        refreshGraph(sections, detailMap, childrenMap);
      }

      // Load children for all visible nodes in parallel
      const childrenResults = await Promise.all(
        uniqueItems.map(async (item) => {
          try {
            const response = await apiClient.listMemoryChildren(item.uri ?? item.node_id);
            return { nodeId: item.node_id, children: response.children };
          } catch {
            return { nodeId: item.node_id, children: [] as MemoryChildEntry[] };
          }
        })
      );

      if (requestToken !== token) {
        return;
      }

      for (const { nodeId, children } of childrenResults) {
        childrenMap.set(nodeId, children);
      }

      refreshGraph(sections, detailMap, childrenMap);

      if (failedKeys.length > 0) {
        graphNotice = `有 ${failedKeys.length} 个节点详情未加载，已先跳过。`;
      }
    } catch (error) {
      if (requestToken !== token) {
        return;
      }
      loadedSignature = "";
      graphError = error instanceof Error ? error.message : "Failed to load memory graph";
    } finally {
      if (requestToken === token) {
        graphLoading = false;
      }
    }
  }

  onMount(() => {
    memoryGraphDebug("initialized", {
      sections: memorySections.length
    });
  });

  onDestroy(() => {
    memoryGraphDebug("destroyed", {
      requestToken,
      graphNodes: graphNodes.length
    });
  });
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === "Escape") {
      memoryGraphDebug("Escape pressed");
      handleClose();
    }
  }}
/>

<div
  class="graph-modal-backdrop"
  role="presentation"
  onclick={(event) => {
    if (event.currentTarget !== event.target) {
      memoryGraphDebug("backdrop click ignored from inner target");
      return;
    }
    memoryGraphDebug("backdrop click");
    handleClose(event);
  }}
>
  <div
    class="graph-modal"
    role="dialog"
    aria-modal="true"
    aria-label="Memory Graph"
    tabindex="-1"
    onclick={(event) => event.stopPropagation()}
    onkeydown={(event) => event.stopPropagation()}
    onpointerdown={(event) => event.stopPropagation()}
  >
    <div class="graph-modal-header">
      <div>
        <p class="header-eyebrow">Memory</p>
        <h3>Memory Graph</h3>
        <p class="header-subtitle">点击节点查看 routes、keywords、metadata 与关系参数。</p>
      </div>
      <button
        class="close-btn"
        type="button"
        onclick={(event) => {
          memoryGraphDebug("close button click");
          handleClose(event);
        }}
        aria-label="关闭"
      >
        <X size={18} strokeWidth={2} />
      </button>
    </div>

    <div
      class="graph-modal-body"
      role="presentation"
      bind:this={graphViewport}
      onpointerdown={(event) => {
        activePopover = null;
        beginPan(event);
      }}
      onpointermove={updatePan}
      onpointerup={endPan}
      onpointercancel={endPan}
      onwheel={handleViewportWheel}
    >
      {#if graphError}
        <div class="graph-empty error">{graphError}</div>
      {:else if graphLoading && graphNodes.length === 0}
        <div class="graph-empty">正在构建记忆图谱…</div>
      {:else if graphNodes.length === 0}
        <div class="graph-empty">当前没有可显示的记忆节点。</div>
      {:else}
        <div class="graph-controls">
          <button class="control-btn" type="button" onclick={zoomOut} aria-label="缩小">
            -
          </button>
          <button class="control-readout" type="button" onclick={resetView} aria-label="重置视图">
            {Math.round(zoom * 100)}%
          </button>
          <button class="control-btn" type="button" onclick={zoomIn} aria-label="放大">
            +
          </button>
        </div>

        {#if graphLoading || graphNotice}
          <div class="graph-status-chip">
            {#if graphLoading}
              正在补充节点详情…
            {:else if graphNotice}
              {graphNotice}
            {/if}
          </div>
        {/if}

        <div
          class="graph-stage"
          class:is-panning={isPanning}
          style={`width:${scaledCanvasWidth}px; height:${scaledCanvasHeight}px;`}
        >
          <svg
            class="graph-svg"
            viewBox={`0 0 ${canvasWidth} ${canvasHeight}`}
            preserveAspectRatio="xMinYMin meet"
          >
            {#each depthColumns as column (column.depth)}
              <g>
                <line
                  class="tree-column-guide"
                  x1={column.x + nodeWidth / 2}
                  y1="72"
                  x2={column.x + nodeWidth / 2}
                  y2={canvasHeight - 64}
                />
                <text
                  class="tree-column-title"
                  x={column.x}
                  y="44"
                >
                  {column.label}
                </text>
              </g>
            {/each}

            {#each graphEdges as edge (edge.renderKey)}
              <path
                class={`graph-edge ${edge.kind} ${edge.isTreePath ? "tree" : "cross"} ${activeNode ? (highlightState.edgeIds.has(edge.renderKey) ? "highlighted" : "muted") : ""}`}
                d={edgePath(edge)}
              />
            {/each}

            {#each graphNodes as node (node.renderKey)}
              {@const colors = sectionColor(node.kind)}
              <g
                class={`graph-node ${activeNode ? (highlightState.nodeIds.has(node.id) ? "highlighted" : "muted") : ""} ${activeNode?.renderKey === node.renderKey ? "active" : ""}`}
                tabindex="0"
                role="button"
                onkeydown={(event) => openNodePopoverFromKeyboard(node, event)}
                onclick={(event) => openNodePopover(node, event)}
              >
                <rect
                  x={node.x}
                  y={node.y}
                  width={node.width}
                  height={node.height}
                  rx="18"
                  fill={colors.fill}
                  stroke={colors.stroke}
                  stroke-width="1.5"
                />
                <text class="node-kicker" x={node.x + 16} y={node.y + 18}>
                  {node.sectionTitle.length > 26 ? `${node.sectionTitle.slice(0, 26)}...` : node.sectionTitle}
                </text>
                <text class="node-title" x={node.x + 16} y={node.y + 42}>
                  {node.label.length > 24 ? `${node.label.slice(0, 24)}...` : node.label}
                </text>
                <text class="node-subtitle" x={node.x + 16} y={node.y + 66}>
                  {node.subtitle.length > 30 ? `${node.subtitle.slice(0, 30)}...` : node.subtitle}
                </text>
              </g>
            {/each}
          </svg>

          {#if activeNode && activePopover}
            <div
              class="graph-popover"
              role="presentation"
              style={popoverStyle}
              onpointerdown={(event) => event.stopPropagation()}
            >
              <div class="popover-head">
                <div>
                  <span class="popover-kicker">{memoryKindLabel(activeNode.kind)}</span>
                  <h4>{activeNode.label}</h4>
                </div>
                <span class="popover-time">
                  {formatMemoryTimestamp(activeNode.detail?.node.updated_at ?? null)}
                </span>
              </div>

              <div class="popover-block">
                <span class="popover-label">URI</span>
                <code>{nodeMeta(activeNode)}</code>
              </div>

              <div class="popover-grid">
                <div class="popover-stat">
                  <span class="popover-label">Routes</span>
                  <strong>{activeNode.detail?.routes.length ?? 0}</strong>
                </div>
                <div class="popover-stat">
                  <span class="popover-label">Keywords</span>
                  <strong>{activeNode.detail?.keywords.length ?? 0}</strong>
                </div>
                <div class="popover-stat">
                  <span class="popover-label">Edges</span>
                  <strong>{activeNode.detail?.edges.length ?? 0}</strong>
                </div>
                <div class="popover-stat">
                  <span class="popover-label">Related</span>
                  <strong>{activeNode.detail?.related_nodes.length ?? 0}</strong>
                </div>
              </div>

              {#if activeNode.detail?.routes.length}
                <div class="popover-block">
                  <span class="popover-label">Routes</span>
                  <div class="pill-list">
                    {#each activeNode.detail.routes.slice(0, 6) as route, index (`${route.id}:${index}`)}
                      <span class="pill">{routeLabel(route)}</span>
                    {/each}
                  </div>
                </div>
              {/if}

              {#if activeNode.detail?.keywords.length}
                <div class="popover-block">
                  <span class="popover-label">Keywords</span>
                  <div class="pill-list">
                    {#each activeNode.detail.keywords.slice(0, 10) as keyword, index (`${keyword.id}:${index}`)}
                      <span class="pill">{keyword.keyword}</span>
                    {/each}
                  </div>
                </div>
              {/if}

              {#if activeNode.detail?.edges.length}
                <div class="popover-block">
                  <span class="popover-label">Relations</span>
                  <div class="relation-list">
                    {#each activeNode.detail.edges.slice(0, 5) as edge, index (`${edge.id}:${index}`)}
                      <div class="relation-row">
                        <strong>{edge.relation_kind}</strong>
                        <span>{edge.visibility} · p{edge.priority}</span>
                        {#if edge.trigger_text}
                          <p>{edge.trigger_text}</p>
                        {/if}
                      </div>
                    {/each}
                  </div>
                </div>
              {/if}

              {#if activeNode.detail?.node.metadata}
                <div class="popover-block">
                  <span class="popover-label">Metadata</span>
                  <pre>{JSON.stringify(activeNode.detail.node.metadata, null, 2)}</pre>
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .graph-modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--settings-z-modal, 94);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 18px;
    background: rgba(10, 14, 20, 0.26);
    backdrop-filter: blur(12px);
  }

  .graph-modal {
    width: min(96vw, 1480px);
    height: min(94vh, 980px);
    display: flex;
    flex-direction: column;
    border-radius: 28px;
    border: 1px solid var(--border-default);
    background: color-mix(in srgb, var(--bg-surface) 90%, white 10%);
    box-shadow: var(--shadow-dropdown);
    overflow: hidden;
  }

  .graph-modal-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 20px;
    padding: 22px 24px 18px;
    border-bottom: 1px solid var(--border-subtle, var(--border-default));
  }

  .header-eyebrow {
    margin: 0 0 6px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .graph-modal-header h3 {
    margin: 0;
    font-size: 24px;
    font-weight: 720;
    color: var(--text-primary);
  }

  .header-subtitle {
    margin: 8px 0 0;
    font-size: 13px;
    line-height: 1.55;
    color: var(--text-secondary);
  }

  .close-btn {
    width: 40px;
    height: 40px;
    border-radius: 14px;
    border: 1px solid var(--border-default);
    background: var(--bg-input);
    color: var(--text-primary);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    flex-shrink: 0;
  }

  .graph-modal-body {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: auto;
    cursor: grab;
    background:
      radial-gradient(circle at top left, color-mix(in srgb, var(--accent-primary) 10%, transparent) 0%, transparent 30%),
      linear-gradient(180deg, color-mix(in srgb, var(--bg-primary) 92%, white 8%), var(--bg-primary));
  }

  .graph-modal-body:active {
    cursor: grabbing;
  }

  .graph-empty {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 40px;
    font-size: 14px;
    color: var(--text-secondary);
  }

  .graph-empty.error {
    color: var(--accent-danger, #c8594f);
  }

  .graph-stage {
    position: relative;
  }

  .graph-stage.is-panning {
    cursor: grabbing;
  }

  .graph-svg {
    display: block;
    width: 100%;
    height: 100%;
  }

  .graph-controls {
    position: sticky;
    top: 18px;
    right: 18px;
    z-index: 2;
    display: flex;
    align-items: center;
    gap: 8px;
    width: fit-content;
    margin: 18px 18px 0 auto;
    padding: 8px;
    border-radius: 16px;
    border: 1px solid var(--border-default);
    background: color-mix(in srgb, var(--bg-surface) 88%, white 12%);
    backdrop-filter: blur(10px);
    box-shadow: 0 10px 30px rgba(18, 24, 34, 0.1);
  }

  .graph-status-chip {
    position: sticky;
    top: 18px;
    left: 18px;
    z-index: 2;
    width: fit-content;
    margin: 12px 0 0 18px;
    padding: 10px 14px;
    border-radius: 999px;
    border: 1px solid var(--border-default);
    background: color-mix(in srgb, var(--bg-surface) 90%, white 10%);
    color: var(--text-secondary);
    font-size: 12px;
    line-height: 1.4;
    box-shadow: 0 10px 30px rgba(18, 24, 34, 0.08);
    backdrop-filter: blur(10px);
  }

  .control-btn,
  .control-readout {
    height: 34px;
    border-radius: 10px;
    border: 1px solid var(--border-subtle, var(--border-default));
    background: var(--bg-input);
    color: var(--text-primary);
    font: inherit;
    cursor: pointer;
  }

  .control-btn {
    width: 34px;
    font-size: 18px;
    line-height: 1;
  }

  .control-readout {
    min-width: 68px;
    padding: 0 12px;
    font-size: 12px;
    font-weight: 650;
  }

  .tree-column-guide {
    stroke: color-mix(in srgb, var(--border-default) 68%, transparent);
    stroke-width: 1;
    stroke-dasharray: 4 8;
  }

  .tree-column-title {
    font-size: 14px;
    font-weight: 680;
    fill: var(--text-secondary);
  }

  .graph-edge {
    fill: none;
    stroke-linecap: round;
    stroke-width: 2;
    opacity: 0.72;
    transition: opacity 0.22s ease, stroke-width 0.22s ease, filter 0.22s ease;
  }

  .graph-edge.relation {
    stroke: color-mix(in srgb, var(--accent-primary) 34%, var(--border-default));
  }

  .graph-edge.related {
    stroke: color-mix(in srgb, var(--text-tertiary) 55%, transparent);
    opacity: 0.48;
  }

  .graph-edge.tree {
    stroke-width: 2.35;
    opacity: 0.82;
  }

  .graph-edge.cross {
    stroke-dasharray: 7 5;
    opacity: 0.34;
  }

  .graph-node {
    cursor: pointer;
    outline: none;
    transition: opacity 0.22s ease, transform 0.22s ease;
  }

  .graph-node rect,
  .graph-node text {
    transition: opacity 0.22s ease, filter 0.22s ease, stroke-width 0.22s ease;
  }

  .graph-node:hover rect,
  .graph-node:focus rect {
    stroke-width: 2.5;
    filter: drop-shadow(0 10px 18px rgba(32, 40, 54, 0.12));
  }

  .graph-node.muted {
    opacity: 0.26;
  }

  .graph-node.highlighted {
    opacity: 1;
  }

  .graph-node.active rect {
    stroke-width: 3;
    filter: drop-shadow(0 14px 22px rgba(32, 40, 54, 0.18));
  }

  .graph-edge.muted {
    opacity: 0.12;
  }

  .graph-edge.highlighted {
    opacity: 1;
    stroke-width: 3.25;
    filter: drop-shadow(0 0 6px color-mix(in srgb, var(--accent-primary) 36%, transparent));
  }

  .node-kicker {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    fill: var(--text-muted);
  }

  .node-title {
    font-size: 14px;
    font-weight: 700;
    fill: var(--text-primary);
  }

  .node-subtitle {
    font-size: 11px;
    fill: var(--text-secondary);
  }

  .graph-popover {
    position: absolute;
    width: 360px;
    max-height: 420px;
    overflow: auto;
    padding: 16px;
    border-radius: 20px;
    border: 1px solid var(--border-default);
    background: color-mix(in srgb, var(--bg-surface) 94%, white 6%);
    box-shadow: 0 24px 60px rgba(18, 24, 34, 0.18);
    backdrop-filter: blur(12px);
  }

  .popover-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  .popover-kicker,
  .popover-label {
    display: inline-block;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .popover-head h4 {
    margin: 6px 0 0;
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .popover-time {
    font-size: 11px;
    color: var(--text-tertiary);
    white-space: nowrap;
  }

  .popover-block {
    margin-top: 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .popover-block code,
  .popover-block pre {
    margin: 0;
    padding: 10px 12px;
    border-radius: 14px;
    border: 1px solid var(--border-subtle, var(--border-default));
    background: var(--bg-input);
    color: var(--text-primary);
    font: inherit;
    font-family: "SF Mono", "JetBrains Mono", monospace;
    font-size: 11px;
    line-height: 1.55;
    word-break: break-word;
    white-space: pre-wrap;
  }

  .popover-grid {
    margin-top: 14px;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
  }

  .popover-stat {
    padding: 12px;
    border-radius: 16px;
    background: var(--bg-input);
    border: 1px solid var(--border-subtle, var(--border-default));
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .popover-stat strong {
    font-size: 16px;
    color: var(--text-primary);
  }

  .pill-list {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .pill {
    padding: 5px 10px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--bg-elevated) 75%, white 25%);
    border: 1px solid var(--border-subtle, var(--border-default));
    font-size: 11px;
    line-height: 1.4;
    color: var(--text-secondary);
  }

  .relation-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .relation-row {
    padding: 10px 12px;
    border-radius: 14px;
    background: var(--bg-input);
    border: 1px solid var(--border-subtle, var(--border-default));
  }

  .relation-row strong {
    font-size: 12px;
    color: var(--text-primary);
  }

  .relation-row span,
  .relation-row p {
    display: block;
    margin: 4px 0 0;
    font-size: 11px;
    line-height: 1.5;
    color: var(--text-secondary);
  }

  @media (max-width: 900px) {
    .graph-modal-backdrop {
      padding: 8px;
    }

    .graph-modal {
      width: 100vw;
      height: 100vh;
      border-radius: 0;
    }

    .graph-popover {
      width: min(320px, calc(100vw - 24px));
    }
  }
</style>
