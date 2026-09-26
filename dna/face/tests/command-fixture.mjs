// Browser-boundary scripts over real native reads. No durable domain writes.
import { errorBody } from './harness.mjs';
import { UNGATED, commandReceipt, commandReply, describeLine, receiptLine, refusalLine, refusalStatus, refusedReply, routeError } from './command-wire.mjs';

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
    source: null, posts: [], gets: [], describes: 0, savedBeforeSend: [], candidateReads: [],
    postMode: 'receipt', getMode: 'receipt', stage: 'recorded',
    verdictStage: 'recorded', corrupt: false, wrongChoice: false,
    ...options,
  };
  // The head's slice for the forwarded session: the ungated two, and a
  // proposal or a verdict while the script seats its principal for it.
  const slice = () => [...UNGATED,
    ...(script.available && script.authorized ? ['PracticePropose'] : []),
    ...(script.reviewProfile && script.reviewAvailable && script.reviewAuthorized ? ['ReviewVerdict'] : [])];
  const receipt = (line, principal = script.principal) => {
    const p = line.payload, verdict = line.call === 'ReviewVerdict';
    const data = {
      command_id: `command/${p.request_id}`, request_id: p.request_id, application_id: service.application,
      operation: verdict ? 'dna.review.verdict' : 'dna.practice.propose', operation_version: '1',
      principal_mode: principal.mode, principal_name: script.corrupt ? 'wrong-person' : principal.name,
      position_id: 'org', target_kind: verdict ? 'dna.review' : 'dna.practice', target_id: verdict ? p.review_id : p.subject_digest,
      subject_digest: p.subject_digest, fingerprint: 'sha256:' + 'c'.repeat(64), reason: script.corrupt ? 'WRONG ACTOR SECRET' : '',
    };
    if (verdict) {
      const stage = script.verdictStage;
      const refused = ['refused', 'refused_other_approved'].includes(stage);
      const accepted = !refused && stage !== 'recorded';
      const settled = ['settled', 'adopted', 'activation_refused', 'refused_other_approved'].includes(stage);
      Object.assign(data, {
        state: refused ? 'refused' : accepted ? 'succeeded' : 'recorded', proposal_state: '',
        verdict_value: script.wrongChoice ? 'revise' : p.verdict, verdict_state: refused ? 'refused' : accepted ? 'accepted' : 'pending',
        review_state: settled ? 'settled' : 'pending', review_outcome: settled ? stage === 'refused_other_approved' ? 'approve' : p.verdict : '',
        review_subject_digest: p.subject_digest,
        activation_state: ['adopted', 'refused_other_approved'].includes(stage) ? 'adopted' : stage === 'activation_refused' ? 'refused' : 'unknown',
        activation_reason: stage === 'activation_refused' ? 'The candidate could not replace its predecessor.' : '',
      });
      if (refused) data.reason = 'This command was refused; another decision may already exist.';
    } else {
      const created = script.stage !== 'recorded';
      const settled = ['approved', 'adopted', 'refused'].includes(script.stage);
      const candidate = created ? script.proposalCandidate || 'sha256:' + 'b'.repeat(64) : '';
      Object.assign(data, {
        state: created ? 'succeeded' : 'recorded', proposal_state: created ? 'created' : 'pending', candidate_digest: candidate,
        review_id: created ? script.proposalReview || 'org/reviews/command/決定' : '',
        review_state: settled ? 'settled' : created ? 'pending' : 'unavailable', review_outcome: settled ? 'approve' : '', review_subject_digest: candidate,
        activation_state: script.stage === 'adopted' ? 'adopted' : script.stage === 'refused' ? 'refused' : created ? 'pending' : 'unknown',
        activation_reason: script.stage === 'refused' ? 'Another candidate replaced this predecessor.' : '',
      });
    }
    return receiptLine(commandReply(commandReceipt(data), script.source));
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
      // No forwarding route at all: the face offers no record command.
      if (!script.profile) payload.data.api.http = '';
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
      if (body.describe === true) { script.describes += 1; return fulfill(200, describeLine(slice())); }
      const principal = structuredClone(script.principal);
      // what the page saved before it sent, read from the page; only then
      // is the POST counted, so a test that waits on `posts` and then
      // navigates never destroys this read mid-flight
      script.savedBeforeSend = await recoveryMetadata(page);
      script.posts.push({ body, headers: request.headers() });
      if (script.waitForPost) await script.waitForPost;
      if (script.postMode === 'lost') return route.abort('failed');
      if (script.postMode === 'stale') return fulfill(200, receiptLine(refusedReply('stale_subject')));
      if (script.postMode === 'identity_changed') return fulfill(200, receiptLine(refusedReply('command_context_changed')));
      if (!slice().includes(body.call)) return fulfill(refusalStatus('unknown'), refusalLine('unknown', body.call));
      return fulfill(200, receipt(body, principal)).catch(() => {});
    }
    script.gets.push(url.searchParams.get('request_id'));
    if (script.getMode === 'unavailable') return fulfill(503, routeError('commands_unavailable', 'the api socket did not answer; retain the request id for lookup'));
    const original = script.posts.find(post => post.body.payload.request_id === url.searchParams.get('request_id'));
    if (script.getMode === 'missing' || !original) return fulfill(200, receiptLine(refusedReply('command_not_found')));
    return fulfill(200, receipt(original.body));
  });
  return script;
}
