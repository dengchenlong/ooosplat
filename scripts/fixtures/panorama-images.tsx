import { createRoot } from 'react-dom/client';
import { useState } from 'react';
import { LanguageProvider } from '../../src/i18n';
import { MediaSourceStatus } from '../../src/components/MediaSourceStatus';
import type { VideoProjection } from '../../src/types/pipeline';
import '../../src/styles.css';

const locale=new URLSearchParams(location.search).get('locale')==='en'?'en':'zh-CN';
localStorage.setItem('ooo-splat-language',locale);
const detection={sourceIdentity:'fixture',confirmedErp:false,requiresChoice:true,supportedErp:true,reason:'',previewJpeg:[]};
function Fixture(){
  const [projection,setProjection]=useState<VideoProjection>('auto');
  return <LanguageProvider><main style={{width:680,maxWidth:'calc(100vw - 40px)',margin:20,display:'grid',gap:16}}>
    {[
      <MediaSourceStatus inputType="images" projection={projection} detection={detection} onProjectionChange={setProjection}/>,
      <MediaSourceStatus inputType="images" projection="equirectangular" detection={{...detection,confirmedErp:true,requiresChoice:false}}/>,
      <MediaSourceStatus inputType="images" projection="perspective" detection={{...detection,requiresChoice:false}}/>,
      <MediaSourceStatus inputType="images" projection="auto" checking/>,
      <MediaSourceStatus inputType="images" projection="equirectangular" historical/>,
    ].map((status,index)=><div className={`source-input-card${index===4?' readonly':''}`} key={index}>
      <div className="path-picker media-path readonly"><span><strong>Ballintoy</strong><small>E:\GaussianSplatting\test\全景图片测试\Ballintoy\Input</small></span></div>{status}
    </div>)}
  </main></LanguageProvider>;
}
createRoot(document.getElementById('root')!).render(<Fixture/>);
(window as unknown as {fixtureReady:boolean}).fixtureReady=true;
