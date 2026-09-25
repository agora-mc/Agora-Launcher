// A content provider answers four questions and never installs anything:
//
//   search   — what exists?
//   project  — tell me about one thing
//   versions — which versions of it are there?
//   resolve  — for this instance, what exactly should be downloaded?
//
// Agora validates every answer, downloads files itself (only from the hosts in
// `network.hosts` count as verified), checks every digest you publish, snapshots
// the instance, and labels what it installed with this provider's id.
//
// A real provider would fetch its catalog with `net.fetchJson(url)` from one of
// its declared hosts. This one keeps its catalog in memory so it runs offline.

const HOST = 'https://downloads.example.org';

// Digests are what Agora checks the downloaded bytes against. Publish SHA-512
// or SHA-256: anything weaker, or a file on a host you did not declare, is
// "unverified content" and needs the user's explicit permission. (These are
// placeholders — 128 hex characters — because the files are not real either.)
const sha512 = (id) => (id.length % 16).toString(16).repeat(128);

const CATALOG = [
  {
    id: 'lantern',
    title: 'Lantern',
    description: 'Dynamic lighting for held torches.',
    author: 'example',
    contentType: 'mod',
    side: 'client',
    downloads: 1200,
    versions: [
      {
        id: 'lantern-2.0.0',
        versionNumber: '2.0.0',
        minecraftVersions: ['1.21.1'],
        loaders: ['fabric'],
        dependencies: [{ projectId: 'lib-core', kind: 'required' }],
      },
    ],
  },
  {
    id: 'lib-core',
    title: 'Lib Core',
    description: 'Shared code other Example Shelf mods need.',
    author: 'example',
    contentType: 'mod',
    side: 'both',
    downloads: 5000,
    versions: [
      {
        id: 'lib-core-1.4.0',
        versionNumber: '1.4.0',
        minecraftVersions: ['1.21.1'],
        loaders: ['fabric'],
        dependencies: [],
      },
    ],
  },
  {
    id: 'cozy-pack',
    title: 'Cozy Pack',
    description: 'A small pack built from the shelf.',
    author: 'example',
    contentType: 'pack',
    side: 'both',
    downloads: 300,
    versions: [
      {
        id: 'cozy-1.0.0',
        versionNumber: '1.0.0',
        minecraftVersions: ['1.21.1'],
        loaders: ['fabric'],
        dependencies: [],
      },
    ],
  },
];

const summary = (p) => ({
  id: p.id,
  title: p.title,
  description: p.description,
  author: p.author,
  contentType: p.contentType,
  downloads: p.downloads,
  pageUrl: `${HOST}/projects/${p.id}`,
  minecraftVersions: [...new Set(p.versions.flatMap((v) => v.minecraftVersions))],
  loaders: [...new Set(p.versions.flatMap((v) => v.loaders))],
});

const find = (id) => {
  const project = CATALOG.find((p) => p.id === id);
  if (!project) throw new Error(`no project ${id}`);
  return project;
};

const fits = (version, mc, loader) =>
  (!mc || version.minecraftVersions.includes(mc)) && (!loader || version.loaders.includes(loader));

export async function search(request) {
  const query = (request.query || '').toLowerCase();
  const side = (request.filters.side || [])[0];
  let hits = CATALOG.filter(
    (p) =>
      (!query || p.title.toLowerCase().includes(query)) &&
      (!request.contentType || p.contentType === request.contentType) &&
      (!side || p.side === side),
  );
  if (request.sort === 'downloads') hits = [...hits].sort((a, b) => b.downloads - a.downloads);
  const page = hits.slice(request.offset, request.offset + request.limit);
  return {
    items: page.map(summary),
    total: hits.length,
    hasMore: request.offset + page.length < hits.length,
  };
}

export async function project({ projectId }) {
  const p = find(projectId);
  return {
    project: summary(p),
    body: `${p.title} is an example project. Nothing here is real.`,
    license: 'MIT',
    links: [{ label: 'Project page', url: `${HOST}/projects/${p.id}` }],
  };
}

export async function versions({ projectId, minecraftVersion, loader }) {
  const p = find(projectId);
  return {
    versions: p.versions
      .filter((v) => fits(v, minecraftVersion, loader))
      .map((v) => ({
        id: v.id,
        name: `${p.title} ${v.versionNumber}`,
        versionNumber: v.versionNumber,
        channel: 'release',
        minecraftVersions: v.minecraftVersions,
        loaders: v.loaders,
        dependencies: v.dependencies,
      })),
  };
}

export async function resolve({ projectId, versionId, minecraftVersion, loader }) {
  const p = find(projectId);
  const version = versionId
    ? p.versions.find((v) => v.id === versionId)
    : p.versions.find((v) => p.contentType === 'pack' || fits(v, minecraftVersion, loader));
  if (!version) throw new Error(`${p.title} has no version for ${minecraftVersion} / ${loader}`);

  if (p.contentType === 'pack') {
    // A pack is a list of files and where each one goes. Paths must sit under
    // mods/, config/, resourcepacks/, shaderpacks/, datapacks/, defaultconfigs/
    // or kubejs/ — Agora refuses anything else.
    return {
      kind: 'pack',
      name: p.title,
      versionId: version.id,
      versionNumber: version.versionNumber,
      minecraftVersion: version.minecraftVersions[0],
      loader: version.loaders[0],
      loaderVersion: '0.16.5',
      files: ['lib-core', 'lantern'].map((id) => {
        const v = find(id).versions[0];
        return {
          path: `mods/${id}-${v.versionNumber}.jar`,
          download: {
            url: `${HOST}/files/${id}-${v.versionNumber}.jar`,
            filename: `${id}-${v.versionNumber}.jar`,
            hashes: { sha512: sha512(id) },
          },
        };
      }),
    };
  }

  return {
    kind: 'file',
    versionId: version.id,
    versionNumber: version.versionNumber,
    contentType: p.contentType,
    file: {
      url: `${HOST}/files/${p.id}-${version.versionNumber}.jar`,
      filename: `${p.id}-${version.versionNumber}.jar`,
      hashes: { sha512: sha512(p.id) },
    },
    dependencies: version.dependencies,
  };
}
