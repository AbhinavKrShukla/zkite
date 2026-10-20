package dev.zkite.runtime;

import android.app.Activity;
import android.graphics.Color;
import android.content.res.ColorStateList;
import android.os.Handler;
import android.view.*;
import android.widget.*;
import org.json.*;

/** Configurable Android controls drawn over the video TextureView. */
final class MediaOverlay {
    interface Player {
        boolean playing();
        int position();
        int duration();
        void toggle();
        void seek(int milliseconds);
        void speed(float value);
        void mute(boolean value);
        void close();
        boolean fullscreen();
        void toggleFullscreen();
        void error(RuntimeException error);
    }
    private final Handler handler = new Handler();
    private LinearLayout panel;
    private final Player player;
    private final int hideMs;
    private final SeekBar seek;
    private final TextView time, toggle;
    private TextView fullscreen;
    private boolean dragging, muted, closed;
    private int speedIndex;
    private final Runnable hide = () -> panel.setVisibility(View.GONE);
    private final Runnable tick = new Runnable() {
        public void run() {
            if(closed) return;
            if(panel.getVisibility()==View.VISIBLE) {
                try {
                int duration=player.duration(), position=player.position();
                if(!dragging) { seek.setMax(Math.max(1,duration)); seek.setProgress(position); }
                time.setText(clock(position)+" / "+clock(duration));
                toggle.setText(player.playing() ? "Pause" : "Play");
                if(fullscreen!=null) fullscreen.setText(player.fullscreen() ? "Exit fullscreen" : "Fullscreen");
                } catch(RuntimeException error) { player.error(error); }
            }
            handler.postDelayed(this,250);
        }
    };
    MediaOverlay(Activity activity, FrameLayout parent, JSONObject style, Player player) throws JSONException {
        this.player=player;
        hideMs=style.optInt("auto_hide_ms",3000);
        int foreground=Color.parseColor(style.optString("foreground","#FFFFFF"));
        int accent=Color.parseColor(style.optString("accent","#818CF8"));
        float font=(float)style.optDouble("text_size",14);
        panel=new LinearLayout(activity); panel.setOrientation(LinearLayout.VERTICAL);
        panel.setBackgroundColor(Color.parseColor(style.optString("background","#CC111827")));
        int pad=(int)(style.optDouble("padding",12)*activity.getResources().getDisplayMetrics().density);
        panel.setPadding(pad,pad,pad,pad);
        FrameLayout.LayoutParams lp=new FrameLayout.LayoutParams(-1,-2,style.optString("placement","bottom").equals("top") ? Gravity.TOP : Gravity.BOTTOM);
        parent.addView(panel,lp);
        time=label(activity,"",foreground,font);
        LinearLayout header=new LinearLayout(activity);
        header.setGravity(Gravity.CENTER_VERTICAL);
        if(style.optBoolean("show_time",true)) header.addView(time,new LinearLayout.LayoutParams(0,-2,1));
        else header.addView(new View(activity),new LinearLayout.LayoutParams(0,0,1));
        if(style.optBoolean("show_fullscreen",true)) fullscreen=button(activity,header,"Fullscreen",foreground,font,player::toggleFullscreen);
        panel.addView(header);
        seek=new SeekBar(activity); seek.setProgressTintList(ColorStateList.valueOf(accent)); seek.setThumbTintList(ColorStateList.valueOf(accent));
        seek.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            public void onStartTrackingTouch(SeekBar bar) { dragging=true; handler.removeCallbacks(hide); }
            public void onProgressChanged(SeekBar bar,int value,boolean user) {}
            public void onStopTrackingTouch(SeekBar bar) { runAction(()->player.seek(bar.getProgress())); dragging=false; show(); }
        });
        if(style.optBoolean("show_seek",true)) panel.addView(seek);
        HorizontalScrollView scroll=new HorizontalScrollView(activity);
        LinearLayout row=new LinearLayout(activity); scroll.addView(row); panel.addView(scroll);
        int skip=style.optInt("skip_seconds",10)*1000;
        if(style.optBoolean("show_skip",true)) button(activity,row,"−"+skip/1000,foreground,font,()->player.seek(Math.max(0,player.position()-skip)));
        toggle=button(activity,row,"Play",foreground,font,player::toggle);
        if(style.optBoolean("show_skip",true)) button(activity,row,"+"+skip/1000,foreground,font,()->player.seek(Math.min(player.duration(),player.position()+skip)));
        if(style.optBoolean("show_mute",true)) {
            final TextView mute=button(activity,row,"Mute",foreground,font,()->{});
            mute.setOnClickListener(v->{ muted=!muted; runAction(()->player.mute(muted)); mute.setText(muted ? "Unmute" : "Mute"); show(); });
        }
        if(style.optBoolean("show_speed",true)) {
            final TextView speed=button(activity,row,"1×",foreground,font,()->{});
            speed.setOnClickListener(v->{ float[] rates={1f,1.25f,1.5f,2f,.5f}; speedIndex=(speedIndex+1)%rates.length; runAction(()->player.speed(rates[speedIndex])); speed.setText(rates[speedIndex]+"×"); show(); });
        }
        if(style.optBoolean("show_close",true)) button(activity,row,"Close",foreground,font,player::close);
        parent.setOnClickListener(v -> { if(panel.getVisibility()==View.VISIBLE) panel.setVisibility(View.GONE); else show(); });
        panel.setOnClickListener(v->show());
        handler.post(tick); show();
    }
    private TextView label(Activity activity,String text,int color,float size) {
        TextView v=new TextView(activity); v.setText(text); v.setTextColor(color); v.setTextSize(size); return v;
    }
    private TextView button(Activity activity,LinearLayout row,String text,int color,float size,Runnable action) {
        TextView v=label(activity,text,color,size); int padding=(int)(12*activity.getResources().getDisplayMetrics().density); v.setPadding(padding,padding,padding,padding);
        v.setContentDescription(text); v.setOnClickListener(w->{ runAction(action); if(!closed) show(); }); row.addView(v); return v;
    }
    private void runAction(Runnable action) { try { action.run(); } catch(RuntimeException error) { player.error(error); } }
    private String clock(int ms) { int seconds=Math.max(0,ms/1000); return String.format(java.util.Locale.ROOT,"%d:%02d",seconds/60,seconds%60); }
    void show() { if(closed) return; panel.setVisibility(View.VISIBLE); handler.removeCallbacks(hide); if(hideMs>0) handler.postDelayed(hide,hideMs); }
    void close() { closed=true; handler.removeCallbacksAndMessages(null); panel.setVisibility(View.GONE); }
}
