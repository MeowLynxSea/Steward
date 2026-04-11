<script lang="ts">
  interface Props {
    size?: number;
    decorative?: boolean;
  }

  let { size = 20, decorative = true }: Props = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 1024 1024"
  fill="none"
  xmlns="http://www.w3.org/2000/svg"
  role={decorative ? undefined : "img"}
  aria-hidden={decorative}
  aria-label={decorative ? undefined : "Steward"}
>
  <defs>
    <linearGradient id="steward-bg" x1="176" y1="144" x2="864" y2="880" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#2B3038" />
      <stop offset="1" stop-color="#12151A" />
    </linearGradient>
    <linearGradient id="steward-facet-top" x1="360" y1="236" x2="620" y2="518" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#F8EFDF" />
      <stop offset="1" stop-color="#E2D3BF" />
    </linearGradient>
    <linearGradient id="steward-facet-right" x1="504" y1="320" x2="776" y2="684" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#A4ACB8" />
      <stop offset="1" stop-color="#7B848F" />
    </linearGradient>
    <linearGradient id="steward-facet-bottom" x1="332" y1="476" x2="696" y2="792" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#D2A051" />
      <stop offset="1" stop-color="#B8802F" />
    </linearGradient>
    <linearGradient id="steward-facet-left" x1="236" y1="360" x2="518" y2="620" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#E4CC9A" />
      <stop offset="1" stop-color="#C7A56A" />
    </linearGradient>
  </defs>

  <rect x="128" y="128" width="768" height="768" rx="224" fill="url(#steward-bg)" />
  <rect x="144" y="144" width="736" height="736" rx="208" stroke="#D1A357" stroke-opacity="0.22" stroke-width="4" />

  <polygon points="512,214 742,444 512,558 282,444" fill="url(#steward-facet-top)" />
  <polygon points="810,512 580,742 466,512 580,282" fill="url(#steward-facet-right)" />
  <polygon points="512,810 282,580 512,466 742,580" fill="url(#steward-facet-bottom)" />
  <polygon points="214,512 444,282 558,512 444,742" fill="url(#steward-facet-left)" />
  <polygon points="512,408 616,512 512,616 408,512" fill="#1A1E24" />
</svg>
