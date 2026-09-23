// Browser-boundary scripts over real native reads. No durable domain writes.
import { errorBody } from './harness.mjs';

export const STORAGE_PREFIX = 'face.practice-recovery.v1:';
export async function recoveryMetadata(page) {
  return page.evaluate(prefix => Object.entries(localStorage)
    .filter(([key]) => key.startsWith(prefix))
    .map(([key, value]) => ({ key, value: JSON.parse(value) })), STORAGE_PREFIX);
}

export async function scriptedCommands(page, service, options = {}) {
  const script = {
    profile: true, available: true, authorized: true, principal: null,
    reviewProfile: false, reviewAvailable: true, reviewAuthorized: true,
    authLost: false, readsUnavailable: false, reviewsUnavailable: false,
    source: null, posts: [], gets: [], savedBeforeSend: [], candidateReads: [],
    postMode: 'receipt', getMode: 'receipt', stage: 'recorded',
    verdictStage: 'recorded', corrupt: false, wrongChoice: false,
    ...options,
  };
  const receipt = (request, principal = script.principal) => {
    const data = {
      command_id: `command/${request.request_id}`, request_id: request.request_id,
      application_id: service.application, operation: request.operation, operation_version: '1',
      principal: script.corrupt ? { mode: 'local', name: 'wrong-person' } : principal,
      context: { application_id: service.application, position_id: 'org' },
      target: request.target, subject_digest: request.preconditions.subject_digest,
      fingerprint: 'sha256:' + 'c'.repeat(64), reason: script.corrupt ? 'WRONG ACTOR SECRET' : '',
    };
    if (request.operation === 'dna.review.verdict') {
      const stage = script.verdictStage;
      const refused = ['refused', 'refused_other_approved'].includes(stage);
      const accepted = !refused && stage !== 'recorded';
      const settled = ['settled', 'adopted', 'activation_refused', 'refused_other_approved'].includes(stage);
      Object.assign(data, {
        state: refused ? 'refused' : accepted ? 'succeeded' : 'recorded',
        verdict: { value: script.wrongChoice ? 'revise' : request.arguments.verdict,
          state: refused ? 'refused' : accepted ? 'accepted' : 'pending' },
        review: { state: settled ? 'settled' : 'pending',
          outcome: settled ? stage === 'refused_other_approved' ? 'approve' : request.arguments.verdict : '',
          subject_digest: request.preconditions.subject_digest },
        activation: { state: ['adopted', 'refused_other_approved'].includes(stage) ? 'adopted' : stage === 'activation_refused' ? 'refused' : 'unknown',
          reason: stage === 'activation_refused' ? 'The candidate could not replace its predecessor.' : '' },
      });
      if (refused) data.reason = 'This command was refused; another decision may already exist.';
    } else {
      const created = script.stage !== 'recorded';
      const settled = ['approved', 'adopted', 'refused'].includes(script.stage);
      const candidate = created ? script.proposalCandidate || 'sha256:' + 'b'.repeat(64) : '';
      Object.assign(data, {
        state: created ? 'succeeded' : 'recorded',
        proposal: { state: created ? 'created' : 'pending', candidate_digest: candidate,
          review_id: created ? script.proposalReview || 'org/reviews/command/決定' : '' },
        review: { state: settled ? 'settled' : created ? 'pending' : 'unavailable',
          outcome: settled ? 'approve' : '', subject_digest: candidate },
        activation: { state: script.stage === 'adopted' ? 'adopted' : script.stage === 'refused' ? 'refused' : created ? 'pending' : 'unknown',
          reason: script.stage === 'refused' ? 'Another candidate replaced this predecessor.' : '' },
      });
    }
    return { api_version: 'hale.v1', source: script.source, data };
  };
  await page.route('**/api/hale/v1/**', async route => {
    const request = route.request();
    const url = new URL(request.url());
    const fulfill = (status, value) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(value) });
    if (script.authLost) return fulfill(401, errorBody('unauthenticated', 'Sign in'));
    if (url.pathname.endsWith('/capabilities')) {
      const response = await route.fetch();
      const payload = await response.json();
      script.source = payload.source;
      script.principal ||= payload.data.principal;
      payload.data.principal = script.principal;
      payload.data.writes.practice_propose = script.available && script.authorized;
      payload.data.writes.review_verdict = Boolean(script.reviewProfile || script.reviewWriteOnly) && script.reviewAvailable && script.reviewAuthorized;
      payload.data.read_only = !(payload.data.writes.practice_propose || payload.data.writes.review_verdict);
      if (script.profile) payload.data.commands = {
        profile: 'dna.practice.propose.v1', available: script.available, authorized: script.authorized,
        position_id: 'org', recovery: 'record_lifetime', max_text_bytes: '8192', max_rationale_bytes: '2048',
        reason: script.authorized ? '' : 'Current principal cannot propose a practice revision.',
      };
      if (script.reviewProfile) payload.data.review_commands = {
        profile: 'dna.review.verdict.v1', available: script.reviewAvailable, authorized: script.reviewAuthorized,
        position_id: 'org', recovery: 'record_lifetime', max_comment_bytes: '2048',
        reason: script.reviewAuthorized ? '' : 'Current principal cannot submit a Review verdict.',
      };
      return fulfill(200, payload);
    }
    if (script.readsUnavailable && url.pathname.endsWith('/dna/practices')) return fulfill(503, errorBody('source_unavailable', 'Practice reads unavailable'));
    if (script.reviewsUnavailable && url.pathname.endsWith('/dna/reviews')) return fulfill(503, errorBody('source_unavailable', 'Review reads unavailable'));
    if (url.pathname.endsWith('/dna/reviews') && (script.reviewMutation || script.reviewApprovers || script.reviewAuthority || script.missingReviewFacts)) {
      const response = await route.fetch();
      const payload = await response.json();
      for (const review of payload.data.items) if (review.id === service.pending_review) {
        if (script.reviewMutation) review.is_mutation = true;
        if (script.reviewApprovers) review.approvers = script.reviewApprovers;
        if (script.reviewAuthority) review.required_authority = script.reviewAuthority;
        if (script.missingReviewFacts) { delete review.is_mutation; delete review.approvers; }
      }
      return fulfill(200, payload);
    }
    if (url.pathname.endsWith('/dna/practices') && url.searchParams.get('id') === service.pending_practice) {
      script.candidateReads.push(url);
      if (script.candidateMismatch) {
        const response = await route.fetch();
        const payload = await response.json();
        payload.data.items[0].review_id = 'another-review';
        return fulfill(200, payload);
      }
    }
    if (!url.pathname.endsWith('/commands')) return route.continue();
    if (request.method() === 'POST') {
      const body = request.postDataJSON();
      const principal = structuredClone(script.principal);
      script.posts.push({ body, headers: request.headers() });
      script.savedBeforeSend = await recoveryMetadata(page);
      if (script.waitForPost) await script.waitForPost;
      if (script.postMode === 'lost') return route.abort('failed');
      if (script.postMode === 'stale') return fulfill(409, errorBody('stale_subject', 'The subject changed.'));
      if (script.postMode === 'identity_changed') return fulfill(409, errorBody('command_context_changed', 'Authenticated identity changed.'));
      const result = receipt(body, principal);
      return fulfill(['succeeded', 'refused', 'failed'].includes(result.data.state) ? 200 : 202, result).catch(() => {});
    }
    script.gets.push(url.searchParams.get('request_id'));
    if (script.getMode === 'unavailable') return fulfill(503, errorBody('commands_unavailable', 'Receipt source unavailable'));
    const original = script.posts.find(post => post.body.request_id === url.searchParams.get('request_id'));
    if (script.getMode === 'missing' || !original) return fulfill(404, errorBody('command_not_found', 'No accepted request found'));
    return fulfill(200, receipt(original.body));
  });
  return script;
}
