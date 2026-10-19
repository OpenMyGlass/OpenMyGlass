package org.openmyglass.android.nav

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.IBinder
import net.osmand.aidlapi.IOsmAndAidlCallback
import net.osmand.aidlapi.IOsmAndAidlInterface
import net.osmand.aidlapi.navigation.ADirectionInfo
import net.osmand.aidlapi.navigation.ANavigationUpdateParams
import net.osmand.aidlapi.navigation.ANavigationVoiceRouterMessageParams
import net.osmand.aidlapi.navigation.OnVoiceNavigationParams

/** The subset of OsmAnd's AIDL API we use, behind an interface so the source can be unit-tested. */
interface OsmAndApi {
    fun registerNavigation(callback: (distanceTo: Int, turnType: Int) -> Unit): Long
    fun registerVoice(callback: (played: List<String>) -> Unit): Long
    fun unregister(id: Long)
}

/** Binds OsmAnd's AIDL service; [bind] returns false if no OsmAnd build is installed. */
interface OsmAndBinder {
    /** [onDisconnected] gets `bindingDied = true` when Android will not reconnect this binding by itself. */
    fun bind(onConnected: (OsmAndApi) -> Unit, onDisconnected: (bindingDied: Boolean) -> Unit): Boolean
    fun unbind()
}

class AndroidOsmAndBinder(private val context: Context) : OsmAndBinder {
    private var connection: ServiceConnection? = null

    override fun bind(onConnected: (OsmAndApi) -> Unit, onDisconnected: (bindingDied: Boolean) -> Unit): Boolean {
        val conn = object : ServiceConnection {
            override fun onServiceConnected(name: ComponentName, service: IBinder) =
                onConnected(AidlOsmAndApi(IOsmAndAidlInterface.Stub.asInterface(service)))
            override fun onServiceDisconnected(name: ComponentName) = onDisconnected(false)
            override fun onBindingDied(name: ComponentName) = onDisconnected(true)
            override fun onNullBinding(name: ComponentName) = onDisconnected(true)
        }
        for (pkg in PACKAGES) {
            val intent = Intent(SERVICE_ACTION).setPackage(pkg)
            if (context.bindService(intent, conn, Context.BIND_AUTO_CREATE)) {
                connection = conn
                return true
            }
            runCatching { context.unbindService(conn) }
        }
        return false
    }

    override fun unbind() {
        connection?.let { runCatching { context.unbindService(it) } }
        connection = null
    }

    companion object {
        /** OsmAnd's AIDL service action (OsmAnd/AndroidManifest.xml, `net.osmand.aidl.OsmandAidlServiceV2`). */
        const val SERVICE_ACTION = "net.osmand.aidl.OsmandAidlServiceV2"
        /** OsmAnd+ / F-Droid build first, then the free and the nightly build. */
        val PACKAGES = listOf("net.osmand.plus", "net.osmand", "net.osmand.dev")

        /**
         * Asks OsmAnd for the "navigation" permission group (registerForNavUpdates, registerForVoiceRouterMessages).
         * Start it with startActivityForResult; OsmAnd shows "Allow … to use OsmAnd?"
         * (net.osmand.aidl.AidlPermissionRequestActivity).
         */
        fun permissionRequestIntent(context: Context): Intent? = PACKAGES
            .map { Intent("net.osmand.aidl.REQUEST_PERMISSIONS").setPackage(it).putExtra("groups", arrayOf("navigation")) }
            .firstOrNull { it.resolveActivity(context.packageManager) != null }
    }
}

private class AidlOsmAndApi(private val osmand: IOsmAndAidlInterface) : OsmAndApi {
    private enum class Kind { NAVIGATION, VOICE }
    private val kinds = mutableMapOf<Long, Kind>()

    override fun registerNavigation(callback: (distanceTo: Int, turnType: Int) -> Unit): Long {
        val params = ANavigationUpdateParams().apply { setSubscribeToUpdates(true) }
        val id = osmand.registerForNavigationUpdates(params, object : CallbackAdapter() {
            override fun updateNavigationInfo(directionInfo: ADirectionInfo) =
                callback(directionInfo.distanceTo, directionInfo.turnType)
        })
        kinds[id] = Kind.NAVIGATION
        return id
    }

    override fun registerVoice(callback: (played: List<String>) -> Unit): Long {
        val params = ANavigationVoiceRouterMessageParams().apply { setSubscribeToUpdates(true) }
        val id = osmand.registerForVoiceRouterMessages(params, object : CallbackAdapter() {
            override fun onVoiceRouterNotify(params: OnVoiceNavigationParams) = callback(params.played.orEmpty())
        })
        kinds[id] = Kind.VOICE
        return id
    }

    override fun unregister(id: Long) {
        when (kinds.remove(id)) {
            Kind.NAVIGATION -> osmand.registerForNavigationUpdates(
                ANavigationUpdateParams().apply { setSubscribeToUpdates(false); callbackId = id }, null)
            Kind.VOICE -> osmand.registerForVoiceRouterMessages(
                ANavigationVoiceRouterMessageParams().apply { setSubscribeToUpdates(false); callbackId = id }, null)
            null -> Unit
        }
    }
}

/** No-op implementation of every callback; subclasses override the ones they need. */
private open class CallbackAdapter : IOsmAndAidlCallback.Stub() {
    override fun onSearchComplete(resultSet: MutableList<net.osmand.aidlapi.search.SearchResult>?) = Unit
    override fun onUpdate() = Unit
    override fun onAppInitialized() = Unit
    override fun onGpxBitmapCreated(bitmap: net.osmand.aidlapi.gpx.AGpxBitmap?) = Unit
    override fun updateNavigationInfo(directionInfo: ADirectionInfo) = Unit
    override fun onContextMenuButtonClicked(buttonId: Int, pointId: String?, layerId: String?) = Unit
    override fun onVoiceRouterNotify(params: OnVoiceNavigationParams) = Unit
    override fun onKeyEvent(params: android.view.KeyEvent?) = Unit
    override fun onLogcatMessage(params: net.osmand.aidlapi.logcat.OnLogcatMessageParams?) = Unit
}
