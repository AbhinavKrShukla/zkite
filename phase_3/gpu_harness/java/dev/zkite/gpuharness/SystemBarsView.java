package dev.zkite.gpuharness;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.view.View;

/** Paints behind transparent Android system bars, including edge-to-edge targets. */
final class SystemBarsView extends View {
    private final Paint paint = new Paint();
    private int statusColor = Color.WHITE;
    private int navigationColor = Color.WHITE;
    private int left, right, bottom;

    SystemBarsView(Context context) {
        super(context);
        setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
    }

    void setColors(int status, int navigation) {
        statusColor = status;
        navigationColor = navigation;
        invalidate();
    }

    void setBarInsets(int leftInset, int rightInset, int bottomInset) {
        left = leftInset;
        right = rightInset;
        bottom = bottomInset;
        invalidate();
    }

    @Override protected void onDraw(Canvas canvas) {
        // The inset SurfaceView covers the center. This view only supplies the
        // uncovered safe-area background; touch and app layout remain unchanged.
        canvas.drawColor(statusColor);
        paint.setColor(navigationColor);
        canvas.drawRect(0, getHeight() - bottom, getWidth(), getHeight(), paint);
        canvas.drawRect(0, 0, left, getHeight(), paint);
        canvas.drawRect(getWidth() - right, 0, getWidth(), getHeight(), paint);
    }
}
