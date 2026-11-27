<script>
  // ChronoSort's own line icons (24x24 grid), drawn for this app.
  let { name, size = 18, title = "" } = $props();

  // Filled file-type badges use a coloured body so kinds are told apart at a glance.
  const KIND = {
    folder: "#e3a73b", image: "#3f8fd8", raw: "#7a5bd6", video: "#d0503f", audio: "#2f9e75",
    document: "#5a6b85", archive: "#a9792f", program: "#4a4f58", sidecar: "#8a8f99", other: "#9aa0a8",
  };
</script>

<svg viewBox="0 0 24 24" width={size} height={size} aria-hidden={title ? undefined : "true"} role={title ? "img" : undefined}
  fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
  {#if title}<title>{title}</title>{/if}
  {#if name === "back"}<path d="M15 5l-7 7 7 7" />
  {:else if name === "forward"}<path d="M9 5l7 7-7 7" />
  {:else if name === "up"}<path d="M12 19V6M6 11l6-6 6 6" />
  {:else if name === "rot-left"}<path d="M7 4L3 8l4 4" /><path d="M3 8h9a7 7 0 1 1-6.3 10" />
  {:else if name === "rot-right"}<path d="M17 4l4 4-4 4" /><path d="M21 8h-9a7 7 0 1 0 6.3 10" />
  {:else if name === "flip-h"}<path d="M12 3v18" stroke-dasharray="2 2" /><path d="M9 6L3 18h6zM15 6l6 12h-6z" />
  {:else if name === "flip-v"}<path d="M3 12h18" stroke-dasharray="2 2" /><path d="M6 9L18 3v6zM6 15l12 6v-6z" />
  {:else if name === "crop"}<path d="M6 2v16h16" /><path d="M2 6h16v16" />
  {:else if name === "resize"}<rect x="3" y="3" width="18" height="18" rx="2" /><path d="M8 16l8-8M11 8h5v5" />
  {:else if name === "save"}<path d="M5 3h11l4 4v14H4V3z" /><path d="M8 3v5h7V3M8 21v-7h8v7" />
  {:else if name === "zoom-in"}<circle cx="10.5" cy="10.5" r="6.5" /><path d="M15.5 15.5L21 21M8 10.5h5M10.5 8v5" />
  {:else if name === "zoom-out"}<circle cx="10.5" cy="10.5" r="6.5" /><path d="M15.5 15.5L21 21M8 10.5h5" />
  {:else if name === "play"}<path d="M7 4.5v15l12.5-7.5z" fill="currentColor" stroke-width="1.4" />
  {:else if name === "pause"}<rect x="6" y="4.5" width="4" height="15" rx="1" fill="currentColor" stroke="none" /><rect x="14" y="4.5" width="4" height="15" rx="1" fill="currentColor" stroke="none" />
  {:else if name === "plus"}<path d="M12 5v14M5 12h14" />
  {:else if name === "slideshow"}<rect x="3" y="4" width="18" height="13" rx="2" /><path d="M10 8.2v5.6l4.8-2.8z" fill="currentColor" stroke-width="1.2" /><path d="M8 21h8M12 17v4" />
  {:else if name === "compare"}<rect x="2.5" y="4.5" width="8.5" height="15" rx="1.5" /><rect x="13" y="4.5" width="8.5" height="15" rx="1.5" /><path d="M5 15l2-3 2 3M15.5 15l2-3 2 3" />
  {:else if name === "tag"}<path d="M3.5 12.2V4.5a1 1 0 0 1 1-1h7.7l8.3 8.3a1.4 1.4 0 0 1 0 2l-6.4 6.4a1.4 1.4 0 0 1-2 0z" /><circle cx="8.2" cy="8.2" r="1.6" />
  {:else if name === "undo"}<path d="M9 14L4 9l5-5" /><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11" />
  {:else if name === "refresh"}<path d="M20 11a8 8 0 1 0-2.3 5.7" /><path d="M20 4v7h-7" />
  {:else if name === "thumbs"}<rect x="3.5" y="3.5" width="7" height="7" rx="1.5" /><rect x="13.5" y="3.5" width="7" height="7" rx="1.5" /><rect x="3.5" y="13.5" width="7" height="7" rx="1.5" /><rect x="13.5" y="13.5" width="7" height="7" rx="1.5" />
  {:else if name === "tiles"}<rect x="3.5" y="4" width="6" height="6" rx="1.2" /><path d="M12.5 5.5h8M12.5 8.5h5" /><rect x="3.5" y="14" width="6" height="6" rx="1.2" /><path d="M12.5 15.5h8M12.5 18.5h5" />
  {:else if name === "list"}<path d="M8 6h12M8 12h12M8 18h12" /><circle cx="4.5" cy="6" r=".9" fill="currentColor" /><circle cx="4.5" cy="12" r=".9" fill="currentColor" /><circle cx="4.5" cy="18" r=".9" fill="currentColor" />
  {:else if name === "details"}<rect x="3.5" y="4" width="17" height="16" rx="1.5" /><path d="M3.5 9h17M3.5 14h17M10 4v16" />
  {:else if name === "tree-left"}<rect x="3.5" y="4" width="17" height="16" rx="1.5" /><path d="M9.5 4v16" /><path d="M5.5 8h2M5.5 11h2" />
  {:else if name === "tree-right"}<rect x="3.5" y="4" width="17" height="16" rx="1.5" /><path d="M14.5 4v16" /><path d="M16.5 8h2M16.5 11h2" />
  {:else if name === "preview"}<rect x="3.5" y="4" width="17" height="16" rx="1.5" /><path d="M3.5 14h17" /><circle cx="8" cy="9" r="1.6" />
  {:else if name === "search"}<circle cx="10.5" cy="10.5" r="6" /><path d="M15 15l5 5" />
  {:else if name === "pc"}<rect x="3" y="4" width="18" height="12" rx="1.5" /><path d="M8 20h8M12 16v4" />
  {:else if name === "drive"}<rect x="3" y="7" width="18" height="10" rx="2" /><circle cx="17" cy="12" r="1" fill="currentColor" /><path d="M6 12h6" />
  {:else if name === "usb"}<rect x="7" y="9" width="10" height="12" rx="2" /><path d="M9 9V3.5h6V9M10.5 5.5v1M13.5 5.5v1" />
  {:else if name === "network"}<rect x="3" y="4" width="18" height="8" rx="1.5" /><path d="M12 12v4M5 20h14M12 16v4" />
  {:else if name === "disc"}<circle cx="12" cy="12" r="8.5" /><circle cx="12" cy="12" r="2.2" />
  {:else if name === "home"}<path d="M4 11l8-7 8 7v9H4z" /><path d="M10 20v-5h4v5" />
  {:else if name === "pictures"}<rect x="3" y="5" width="18" height="14" rx="2" /><circle cx="8.5" cy="10" r="1.6" /><path d="M21 16l-5-5-8 8" />
  {:else if name === "desktop"}<rect x="3" y="4" width="18" height="12" rx="1.5" /><path d="M8 20h8" />
  {:else if name === "downloads"}<path d="M12 4v11M7 10l5 5 5-5M5 20h14" />
  {:else if name === "documents"}<path d="M6 3h8l4 4v14H6z" /><path d="M14 3v4h4M9 12h6M9 16h6" />
  {:else if name === "videos"}<rect x="3" y="6" width="13" height="12" rx="2" /><path d="M16 10l5-3v10l-5-3z" />
  {:else if name === "chev-right"}<path d="M9 6l6 6-6 6" />
  {:else if name === "chev-down"}<path d="M6 9l6 6 6-6" />
  {:else if name === "sort"}<path d="M7 4v16M4 17l3 3 3-3M14 6h7M14 11h5M14 16h3" />
  {:else if name === "sort-asc"}<path d="M7 20V4M4 7l3-3 3 3" />
  {:else if name === "sort-desc"}<path d="M7 4v16M4 17l3 3 3-3" />
  {:else if name === "open"}<path d="M4 6h6l2 2h8v11H4z" />
  {:else if name === "explorer"}<path d="M4 6h6l2 2h8v11H4z" /><path d="M14 13h5M17 10l3 3-3 3" />
  {:else if name === "calendar"}<rect x="3" y="5" width="18" height="16" rx="2.5" /><path d="M3 10h18M8 3v4M16 3v4" />
  {:else if name === "dup"}<rect x="3" y="3" width="13" height="13" rx="2.5" /><rect x="8" y="8" width="13" height="13" rx="2.5" />
  {:else if name === "people"}<circle cx="9" cy="8" r="3.5" /><path d="M2.5 20c0-3.6 2.9-6 6.5-6s6.5 2.4 6.5 6" /><circle cx="17" cy="9" r="2.7" /><path d="M16.5 14.2c2.9.2 5 2.2 5 5.3" />
  {:else if name === "pin"}<path d="M12 21s-6.5-6.1-6.5-11.2a6.5 6.5 0 0 1 13 0C18.5 14.9 12 21 12 21z" /><circle cx="12" cy="9.8" r="2.4" />
  {:else if name === "fullscreen"}<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" />
  {:else if name === "close"}<path d="M6 6l12 12M18 6L6 18" />
  {:else if name === "tools"}<rect x="3" y="3" width="7.5" height="7.5" rx="2" /><rect x="13.5" y="3" width="7.5" height="7.5" rx="2" /><rect x="3" y="13.5" width="7.5" height="7.5" rx="2" /><rect x="13.5" y="13.5" width="7.5" height="7.5" rx="2" />
  {:else if name === "browse"}<path d="M3 6h7l2 2h9v12H3z" /><path d="M3 11h18" />
  {:else if name in KIND}
    <!-- file-type badge -->
    {#if name === "folder"}
      <path d="M2.5 6.5a2 2 0 0 1 2-2h5l2 2.2h8a2 2 0 0 1 2 2V18a2 2 0 0 1-2 2h-15a2 2 0 0 1-2-2z" fill={KIND.folder} stroke="#b07a1e" stroke-width="1" />
      <path d="M2.5 9.5h19" stroke="#f6d48a" stroke-width="1.2" />
    {:else}
      <path d="M6 2.5h8.5l4.5 4.5v14.5H6z" fill="#fff" stroke={KIND[name]} stroke-width="1.3" />
      <path d="M14.5 2.5V7H19" stroke={KIND[name]} stroke-width="1.3" />
      <rect x="4" y="11" width="13" height="7" rx="1.5" fill={KIND[name]} stroke="none" />
      {#if name === "image" || name === "raw"}<path d="M6.5 16.5l2.5-3 2 2 1.5-1.5 2.5 2.5" stroke="#fff" stroke-width="1.2" />
      {:else if name === "video"}<path d="M9 13v4l3.5-2z" fill="#fff" stroke="none" />
      {:else if name === "audio"}<path d="M9 17v-4l4-1v4" stroke="#fff" stroke-width="1.2" /><circle cx="8.3" cy="17" r=".9" fill="#fff" stroke="none" /><circle cx="12.3" cy="16" r=".9" fill="#fff" stroke="none" />
      {:else if name === "archive"}<path d="M10.5 11v7M9.5 12.5h2M9.5 14.5h2M9.5 16.5h2" stroke="#fff" stroke-width="1" />
      {:else if name === "program"}<path d="M7.5 13.5l2 1.5-2 1.5M11 17h3" stroke="#fff" stroke-width="1.2" />
      {:else}<path d="M7 13.5h7M7 15.8h5" stroke="#fff" stroke-width="1.2" />{/if}
    {/if}
  {/if}
</svg>
