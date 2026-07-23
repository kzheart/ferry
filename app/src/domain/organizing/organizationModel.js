export function existingProposalFor(proposals, backbones) {
  const expected = new Map(backbones.map(item =>
    [`${item.tool}\0${item.id}`, item.fingerprint]));
  return (proposals || []).find(proposal => {
    if (proposal.status === "stale" || proposal.targets?.length !== expected.size) return false;
    return proposal.targets.every(target =>
      expected.get(`${target.tool}\0${target.id}`) === target.fingerprint);
  }) || null;
}

export function generationInput(backbones, sessions, locale) {
  const byIdentity = new Map(sessions.map(session =>
    [`${session.tool}\0${session.id}`, session]));
  return {
    locale,
    sessions: backbones.map(backbone => {
      const source = byIdentity.get(`${backbone.tool}\0${backbone.id}`) || {};
      const pending = new Map((backbone.pending_sources || []).map(segment =>
        [segment.hash, segment.text]));
      return {
        tool: backbone.tool,
        id: backbone.id,
        title: source.title,
        project: source.project,
        updated_at: source.updated_at,
        segments: backbone.segments.map(segment => ({
          hash: segment.hash,
          text: (pending.get(segment.hash) || segment.digest || "").slice(0, 24_000),
          digest: segment.digest,
        })).filter(segment => segment.text),
      };
    }),
  };
}

export function proposalTargets(context, generated) {
  const results = new Map(generated.sessions.map(item =>
    [`${item.tool}\0${item.id}`, item]));
  const clusterByMember = new Map();
  for (const cluster of generated.clusters || []) {
    for (const member of cluster.members || []) {
      clusterByMember.set(`${member.tool}\0${member.id}`, cluster);
    }
  }
  return context.sessions.map(session => {
    const key = `${session.tool}\0${session.id}`;
    const result = results.get(key);
    const cluster = clusterByMember.get(key);
    if (!result) throw new Error(`Missing organization result for ${key}`);
    return {
      tool: session.tool,
      id: session.id,
      fingerprint: session.fingerprint,
      sources: session.segments.map(segment => segment.hash),
      suggested: {
        name: result.title,
        summary: result.summary,
        tags: result.tags,
        dead_candidate: !!result.dead,
        ...(result.dead_reason ? { dead_reason: result.dead_reason } : {}),
        ...(cluster ? { cluster_id: cluster.id, cluster_name: cluster.name } : {}),
      },
    };
  });
}
