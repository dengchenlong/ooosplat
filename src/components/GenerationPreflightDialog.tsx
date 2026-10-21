import { useEffect, useRef, useState } from 'react';
import { X } from 'lucide-react';
import { useI18n } from '../i18n';
import { answerIsValid, type PreflightAnswers, type PreflightQuestion } from '../app/generationPreflight';

export interface PreflightSession { questions: PreflightQuestion[]; finalLabel: string; resolve: (answers: PreflightAnswers | null) => void }
export function GenerationPreflightDialog({ session, onFinish }: { session: PreflightSession; onFinish: (answers: PreflightAnswers | null) => void }) {
  const { t } = useI18n();
  const [index, setIndex] = useState(0);
  const [answers, setAnswers] = useState<PreflightAnswers>({});
  const root = useRef<HTMLElement>(null);
  const finished = useRef(false);
  const question = session.questions[index];
  const finish = (result: PreflightAnswers | null) => { if (!finished.current) { finished.current = true; onFinish(result); } };
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    root.current?.focus();
    return () => { if (previous?.isConnected) previous.focus(); };
  }, []);
  const last = index === session.questions.length - 1;
  return <div className="preflight-backdrop" onKeyDown={event => {
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); finish(null); }
    if (event.key === 'Tab') {
      const elements = Array.from(root.current?.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),[tabindex="0"]') ?? []);
      const first = elements[0], final = elements.at(-1);
      if (event.shiftKey && (document.activeElement === first || document.activeElement === root.current)) { event.preventDefault(); final?.focus(); }
      else if (!event.shiftKey && document.activeElement === final) { event.preventDefault(); first?.focus(); }
    }
  }}>
    <section ref={root} tabIndex={-1} className="preflight-dialog" role="dialog" aria-modal="true" aria-labelledby="preflight-title" aria-describedby="preflight-description">
      <header><div><small>{t('preflight.step', { current: index + 1, total: session.questions.length })}</small><h2 id="preflight-title">{t(question.kind === 'smallImages' ? 'dialog.smallSequenceTitle' : 'preflight.imageOrderTitle')}</h2></div><button type="button" aria-label={t('common.close')} onClick={() => finish(null)}><X size={18} /></button></header>
      <div className="preflight-body" key={question.id}>
        <strong className="preflight-source">{question.draft.inputPath?.split(/[\\/]/).at(-1)}</strong>
          <p id="preflight-description">{t(question.kind === 'smallImages' ? 'dialog.smallSequence' : 'preflight.imageOrderDescription', { count: question.draft.imageSequence?.imageCount ?? 0 })}</p>
          {question.kind === 'smallImages' && <p className="field-note">{t('captureAdvice.overlap')} {t('captureAdvice.coverage')} {t('captureAdvice.consistency')} {t('captureAdvice.aiWarning')}</p>}
          {question.kind === 'imageOrder' ? <div className="preflight-choices" role="radiogroup" aria-labelledby="preflight-title">
            {(['sequential', 'exhaustive'] as const).map(strategy => <label className="preflight-accept" key={strategy}><input type="radio" name={question.id} value={strategy} checked={answers[question.id] === strategy} onChange={() => setAnswers(current => ({ ...current, [question.id]: strategy }))} />{t(strategy === 'sequential' ? 'preflight.ordered' : 'preflight.unordered')}</label>)}
          </div> : <label className="preflight-accept"><input type="checkbox" checked={answers[question.id] === 'accepted'} onChange={event => setAnswers(current => ({ ...current, [question.id]: event.target.checked ? 'accepted' : '' }))} />{t('preflight.acceptWarning')}</label>}
      </div>
      <footer><button type="button" className="secondary-action" onClick={() => finish(null)}>{t('common.cancel')}</button><div>{index > 0 && <button type="button" className="secondary-action" onClick={() => setIndex(value => value - 1)}>{t('preflight.previous')}</button>}<button type="button" className="primary-action" disabled={!answerIsValid(question, answers[question.id]) || (last && !session.questions.every(item => answerIsValid(item, answers[item.id])))} onClick={() => last ? finish(answers) : setIndex(value => value + 1)}>{last ? session.finalLabel : t('preflight.next')}</button></div></footer>
    </section>
  </div>;
}
