import { useId } from 'react';
import { CircleAlert, Globe, LoaderCircle, RotateCcw, TvMinimalPlay } from 'lucide-react';
import { useI18n } from '../i18n';
import type { InputType, ProjectionDetection, VideoProjection } from '../types/pipeline';
import { CompactError } from './CompactError';

interface Props {
  inputType: InputType;
  checking?: boolean;
  error?: string | null;
  detection?: ProjectionDetection | null;
  projection?: VideoProjection | null;
  historical?: boolean;
  disabled?: boolean;
  onProjectionChange?: (value: VideoProjection) => void;
  onRetry?: () => void;
}

/** The connected lower half of the media picker; no independent card or preview. */
export function MediaSourceStatus({ inputType, checking, error, detection, projection, historical, disabled, onProjectionChange, onRetry }: Props) {
  const { t } = useI18n();
  const id = useId();
  if (checking) return <div className="media-source-status checking" role="status" aria-live="polite"><LoaderCircle size={17} className="spin" /><span>{t(inputType === 'video' ? 'source.detectingVideo' : 'source.detectingImages')}</span></div>;
  if (error) return <div className="media-source-status source-analysis-error" role="alert"><CircleAlert size={17} /><div><strong>{t('source.analysisFailed')}</strong><CompactError message={error} /></div>{onRetry && <button className="media-retry" type="button" disabled={disabled} onClick={onRetry}><RotateCcw size={13} />{t('source.retry')}</button>}</div>;
  const images = inputType === 'images';
  const panorama = projection === 'equirectangular';
  const choice = !historical && detection?.requiresChoice;
  const unresolved = choice && (!projection || projection === 'auto');
  const unknown = historical && !images && (!projection || projection === 'auto');
  const showTypeIcon = !images && !unresolved && !unknown;
  const TypeIcon = panorama ? Globe : TvMinimalPlay;
  const title = images ? unresolved ? 'source.chooseImageType' : panorama ? 'source.panoramaImages' : 'source.images' : unknown ? 'source.unknown' : unresolved ? 'source.chooseType' : panorama ? 'panorama.detected' : 'panorama.ordinaryDetected';
  const hint = historical
    ? images ? panorama ? 'source.historyPanoramaImages' : 'source.historyImages' : unknown ? 'source.unknownHint' : panorama ? 'source.historyPanorama' : 'source.historyOrdinary'
    : images ? unresolved ? 'source.ambiguousImages' : panorama ? detection?.confirmedErp ? 'source.confirmedImages' : 'source.manualPanoramaImages' : 'source.imagesHint' : unresolved ? 'panorama.ambiguous' : panorama ? detection?.confirmedErp ? 'panorama.confirmed' : 'source.manualPanorama' : 'panorama.ordinaryHint';
  return <div className={showTypeIcon ? 'media-source-status has-type-icon' : 'media-source-status'} aria-live="polite">
    <div className="media-source-heading">
      {showTypeIcon && <TypeIcon className="media-source-type-icon" size={18} strokeWidth={1.7} aria-hidden="true" />}
      <div className="media-source-copy">
        <strong>{t(title)}</strong>
        <p id={`${id}-hint`}>{t(hint)}</p>
      </div>
    </div>
    {choice && <select id={id} className="video-type-select" aria-label={t(images ? 'source.chooseImageType' : 'source.chooseType')} aria-describedby={`${id}-hint`} required aria-invalid={unresolved || undefined} disabled={disabled} value={projection === 'auto' ? '' : projection ?? ''} onChange={event => onProjectionChange?.((event.target.value || 'auto') as VideoProjection)}>
      <option value="">{t('source.choosePlaceholder')}</option>
      <option value="perspective">{t(images ? 'source.images' : 'panorama.ordinaryDetected')}</option>
      <option value="equirectangular" disabled={!detection?.supportedErp}>{t(images ? 'source.panoramaImages' : 'source.fullPanorama')}</option>
    </select>}
    {!historical && detection?.unsupported && <p className="media-source-warning">{t(images ? 'source.unsupportedImages' : 'panorama.unsupported')}</p>}
  </div>;
}
