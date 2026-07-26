const paths = {
  dashboard: '<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/>',
  home: '<path d="m3 11 9-8 9 8"/><path d="M5 10v10h14V10"/><path d="M9 20v-6h6v6"/>',
  apps: '<rect x="4" y="4" width="6" height="6" rx="1"/><rect x="14" y="4" width="6" height="6" rx="1"/><rect x="4" y="14" width="6" height="6" rx="1"/><path d="M14 17h6M17 14v6"/>',
  backup: '<path d="M3 12a9 9 0 1 0 3-6.7"/><path d="M3 4v5h5"/><path d="M12 7v5l3 2"/>',
  integrations: '<path d="M8 3v3a2 2 0 0 1-2 2H3"/><path d="M16 3v3a2 2 0 0 0 2 2h3"/><path d="M8 21v-3a2 2 0 0 0-2-2H3"/><path d="M16 21v-3a2 2 0 0 1 2-2h3"/><rect x="8" y="8" width="8" height="8" rx="2"/>',
  vault: '<rect x="5" y="3" width="14" height="18" rx="2"/><path d="M9 7h6M9 11h6M9 15h4"/>',
  settings: '<path d="M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Z"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.12 2.12-.06-.06a1.7 1.7 0 0 0-1.88-.34 1.7 1.7 0 0 0-1.03 1.56V20h-3v-.08a1.7 1.7 0 0 0-1.03-1.56 1.7 1.7 0 0 0-1.88.34l-.06.06-2.12-2.12.06-.06A1.7 1.7 0 0 0 5 15.4a1.7 1.7 0 0 0-1.56-1.03H3v-3h.08A1.7 1.7 0 0 0 4.64 10a1.7 1.7 0 0 0-.34-1.88l-.06-.06L6.36 5.94l.06.06a1.7 1.7 0 0 0 1.88.34A1.7 1.7 0 0 0 9.33 4.8V4h3v.08a1.7 1.7 0 0 0 1.03 1.56 1.7 1.7 0 0 0 1.88-.34l.06-.06 2.12 2.12-.06.06A1.7 1.7 0 0 0 17 9.3a1.7 1.7 0 0 0 1.56 1.03H20v3h-.08A1.7 1.7 0 0 0 18.36 15Z"/>',
  cloud: '<path d="M17.5 19H7a5 5 0 0 1-.8-9.94A7 7 0 0 1 19.7 11 4 4 0 0 1 17.5 19Z"/>',
  system: '<rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8M12 17v4"/>',
  check: '<path d="m5 12 4 4L19 6"/>',
  shield: '<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10Z"/><path d="m9 12 2 2 4-4"/>',
  sync: '<path d="M20 7h-5V2"/><path d="M4 17h5v5"/><path d="M5.1 9A7 7 0 0 1 17 5l3 2M18.9 15A7 7 0 0 1 7 19l-3-2"/>',
  storage: '<ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v6c0 1.7 3.6 3 8 3s8-1.3 8-3V5"/><path d="M4 11v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6"/>',
  update: '<path d="M12 3v12"/><path d="m7 10 5 5 5-5"/><path d="M5 21h14"/>',
  model: '<rect x="4" y="5" width="16" height="14" rx="3"/><path d="M9 9h.01M15 9h.01M8 14h8"/><path d="M12 2v3"/>',
  agent: '<path d="M12 3a4 4 0 1 0 0 8 4 4 0 0 0 0-8Z"/><path d="M5 21a7 7 0 0 1 14 0"/><path d="M20 8h2M2 8h2M12 1v2"/>',
  bell: '<path d="M18 8a6 6 0 1 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9"/><path d="M10 21h4"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/>',
  refresh: '<path d="M20 7h-5V2"/><path d="M4 17h5v5"/><path d="M5.1 9A7 7 0 0 1 17 5l3 2M18.9 15A7 7 0 0 1 7 19l-3-2"/>',
  user: '<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>',
  key: '<circle cx="7" cy="17" r="3"/><path d="m9 15 9-9 3 3-2 2 2 2-2 2-2-2-6 6"/>',
  lock: '<rect x="5" y="10" width="14" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/>',
  file: '<path d="M6 2h8l4 4v16H6z"/><path d="M14 2v5h5M9 13h6M9 17h6"/>',
  activity: '<path d="M3 12h4l2-5 4 10 2-5h6"/>',
  cpu: '<rect x="6" y="6" width="12" height="12" rx="2"/><path d="M9 1v3M15 1v3M9 20v3M15 20v3M20 9h3M20 14h3M1 9h3M1 14h3"/>',
  memory: '<rect x="3" y="6" width="18" height="12" rx="2"/><path d="M7 10h2v4H7zM11 10h2v4h-2zM15 10h2v4h-2z"/>',
  network: '<path d="m7 7 5-5 5 5M12 2v14"/><path d="m17 17-5 5-5-5M12 22V8"/>',
  play: '<path d="m8 5 11 7-11 7z"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  external: '<path d="M15 3h6v6M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
  arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  warning: '<path d="M10.3 3.6 2.4 17.2A2 2 0 0 0 4.1 20h15.8a2 2 0 0 0 1.7-2.8L13.7 3.6a2 2 0 0 0-3.4 0Z"/><path d="M12 9v4M12 17h.01"/>',
  download: '<path d="M12 3v12"/><path d="m7 10 5 5 5-5"/><path d="M5 21h14"/>',
  upload: '<path d="M12 21V9"/><path d="m7 14 5-5 5 5"/><path d="M5 3h14"/>',
  restore: '<path d="M3 12a9 9 0 1 0 3-6.7"/><path d="M3 4v5h5"/><path d="M12 7v5l-3 2"/>',
  eye: '<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/>',
  menu: '<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
  logs: '<path d="M4 4h16v16H4z"/><path d="M8 8h8M8 12h8M8 16h5"/>',
  power: '<path d="M12 2v10"/><path d="M18.4 6.6a8 8 0 1 1-12.8 0"/>',
};

export function icon(name, size = 20, className = "") {
  return `<svg class="icon ${className}" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] || paths.dashboard}</svg>`;
}

export function logoMark(size = 40) {
  return `<svg class="logo-mark" width="${size}" height="${size}" viewBox="0 0 48 48" fill="none" aria-hidden="true"><path d="M24 3 42 13v22L24 45 6 35V13L24 3Z" fill="#1460ff"/><path d="m13 18 11-7 11 7v13l-11 6-11-6V18Z" fill="white" fill-opacity=".96"/><path d="M17 21v10h5v-6l2 2 2-2v6h5V21l-7 5-7-5Z" fill="#1460ff"/></svg>`;
}
