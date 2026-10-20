package org.openmyglass.android.service

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch
import org.openmyglass.android.SettingsActivity
import org.openmyglass.android.PhoneHub
import org.openmyglass.android.transport.BridgeMode
import org.openmyglass.android.transport.Settings
import org.openmyglass.android.transport.TransportTargets
import org.openmyglass.android.transport.ProvidedTransport
import org.openmyglass.android.transport.Transports
import org.openmyglass.core.bridge.BridgeManager
import org.openmyglass.core.connection.ConnectionManager
import org.openmyglass.core.connection.ConnectionState
import org.openmyglass.core.connection.LinkState
import org.openmyglass.core.transport.TransportKind

/**
 * Keeps the links to Glass alive while the app is in the background (architecture §4): the bridge to the
 * OpenMyGlass Glass app (navigation) and the companion link to the XE24 firmware. Reconnection is event-driven:
 * backoff up to 60 s, skipped when Bluetooth turns on or the Glass connects at the ACL level.
 */
class OpenMyGlassService : Service() {
    private var scope: CoroutineScope? = null
    private var bridge: BridgeManager? = null
    private var companion: ConnectionManager? = null
    /** Bumped on Bluetooth broadcasts: the notification text depends on them too (issue #43). */
    private val bluetoothTick = MutableStateFlow(0)

    private val bluetoothEvents = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            bluetoothTick.value++
            val event = when (intent.action) {
                BluetoothAdapter.ACTION_STATE_CHANGED ->
                    if (intent.getIntExtra(BluetoothAdapter.EXTRA_STATE, -1) == BluetoothAdapter.STATE_ON) BluetoothEvents.Event.AdapterOn
                    else BluetoothEvents.Event.Other
                BluetoothDevice.ACTION_ACL_CONNECTED -> BluetoothEvents.Event.AclConnected(
                    @Suppress("DEPRECATION") intent.getParcelableExtra<BluetoothDevice>(BluetoothDevice.EXTRA_DEVICE)?.address)
                else -> BluetoothEvents.Event.Other
            }
            if (BluetoothEvents.shouldReconnect(event, Settings(context).load().glassAddress)) {
                bridge?.reconnectNow()
                companion?.reconnectNow()
            }
        }
    }

    /** The phone joining Wi-Fi can make an automatic media sync possible (architecture §5.4). */
    private val networkChanges = object : android.net.ConnectivityManager.NetworkCallback() {
        override fun onCapabilitiesChanged(network: android.net.Network, caps: android.net.NetworkCapabilities) {
            scope?.launch { PhoneHub.media.conditionsChanged() }
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL, "Glass connection", NotificationManager.IMPORTANCE_LOW))
        val filter = IntentFilter().apply {
            addAction(BluetoothAdapter.ACTION_STATE_CHANGED)
            addAction(BluetoothDevice.ACTION_ACL_CONNECTED)
        }
        if (Build.VERSION.SDK_INT >= 33) registerReceiver(bluetoothEvents, filter, RECEIVER_NOT_EXPORTED)
        else registerReceiver(bluetoothEvents, filter)
        runCatching { getSystemService(android.net.ConnectivityManager::class.java).registerDefaultNetworkCallback(networkChanges) }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Enter the foreground first, whatever happens next: a service started with startForegroundService that
        // stops without startForeground crashes the app. Without BLUETOOTH_CONNECT this throws on API 34+
        // (connectedDevice prerequisite), and a sticky restart may be refused the foreground: stop cleanly.
        val foreground = try {
            startForeground(NOTIFICATION_ID, notification("Starting…", ongoing = true), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
            true
        } catch (e: Exception) {
            android.util.Log.w(TAG, "cannot start in the foreground: $e")
            false
        }
        val permitted = hasBluetoothPermission(this)
        if (!foreground || !permitted) {
            if (foreground) stopForeground(STOP_FOREGROUND_REMOVE)
            // Own id: the system removes the foreground notification's id when the service stops
            getSystemService(NotificationManager::class.java).notify(PROBLEM_NOTIFICATION_ID, notification(
                if (permitted) "Could not start, open OpenMyGlass" else
                    ServiceStatus.text(LinkState.Stopped, companionConnected = false, permissionMissing = true), ongoing = false))
            PhoneHub.setBridgeStatus(if (permitted) "service: could not start" else "service: Bluetooth permission needed")
            stopSelf()
            return START_NOT_STICKY
        }
        getSystemService(NotificationManager::class.java).cancel(PROBLEM_NOTIFICATION_ID)
        restartLinks()
        return START_STICKY
    }

    /** (Re)creates both links from the current settings; called on every start, so saving settings applies them. */
    private fun restartLinks() {
        stopLinks()
        PhoneHub.init(applicationContext)
        val s = CoroutineScope(SupervisorJob() + Dispatchers.Default).also { scope = it }
        val settings = Settings(this)
        val bridge = BridgeManager(
            transport = { Transports.create(this, TransportTargets.bridge(settings.load(), Transports.environment(this))) },
            relay = PhoneHub.relay,
            appVersion = appVersion(),
            scope = s,
            onError = { android.util.Log.w(TAG, "bridge link: $it") },
            notifications = PhoneHub.notifications,
            dictation = PhoneHub.dictation,
            media = PhoneHub.media,
        ).also { this.bridge = it }
        val companionState = MutableStateFlow<ConnectionState>(ConnectionState.Disconnected)
        if (settings.load().companionEnabled) {
            companion = ConnectionManager(
                transport = ProvidedTransport(TransportKind.RFCOMM) {
                    Transports.create(this, TransportTargets.companion(settings.load(), Transports.environment(this)))
                },
                phone = { PhoneStates.current(this) },
                scope = s,
            ).also { m -> s.launch { m.state.collect { companionState.value = it } }; m.start() }
        }
        // Bridge off: no loop waking up for nothing (§6)
        if (settings.load().bridgeMode != BridgeMode.OFF) bridge.start()
        s.launch {
            combine(bridge.state, companionState, bluetoothTick) { b, c, _ -> b to (c is ConnectionState.Connected) }.collect { (b, companionUp) ->
                val env = Transports.environment(this@OpenMyGlassService)
                val text = ServiceStatus.text(b, companionUp, permissionMissing = !env.hasBluetoothPermission,
                    unavailableReason = TransportTargets.bridgeUnavailableReason(settings.load(), env))
                getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, notification(text, ongoing = true))
                PhoneHub.setBridgeStatus("bridge: ${describe(b)}" + if (companionUp) " · companion connected" else "")
            }
        }
    }

    private fun describe(state: LinkState<*>): String = when (state) {
        is LinkState.Connected<*> -> "connected"
        is LinkState.WaitingToRetry -> "not reachable, retry ${state.attempt} in ${state.delay}"
        is LinkState.Unavailable -> "unavailable (${state.reason})"
        else -> state.toString().lowercase()
    }

    private fun stopLinks() {
        bridge?.stop(); bridge = null
        companion?.stop(); companion = null
        scope?.cancel(); scope = null
    }

    override fun onDestroy() {
        unregisterReceiver(bluetoothEvents)
        runCatching { getSystemService(android.net.ConnectivityManager::class.java).unregisterNetworkCallback(networkChanges) }
        stopLinks()
        PhoneHub.setBridgeStatus("bridge: service stopped")
        super.onDestroy()
    }

    private fun notification(text: String, ongoing: Boolean): Notification =
        Notification.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
            .setContentTitle("OpenMyGlass")
            .setContentText(text)
            .setOngoing(ongoing)
            .setContentIntent(PendingIntent.getActivity(this, 0, Intent(this, SettingsActivity::class.java),
                PendingIntent.FLAG_IMMUTABLE))
            .build()

    private fun appVersion(): String =
        runCatching { packageManager.getPackageInfo(packageName, 0).versionName }.getOrNull() ?: "dev"

    companion object {
        private const val TAG = "OpenMyGlassService"
        private const val CHANNEL = "links"
        private const val NOTIFICATION_ID = 1
        private const val PROBLEM_NOTIFICATION_ID = 2
        fun hasBluetoothPermission(context: Context) = Build.VERSION.SDK_INT < 31 ||
            context.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED

        /** Starts (or restarts with the saved settings) the service; false if the permission is missing. */
        fun start(context: Context): Boolean {
            if (!hasBluetoothPermission(context)) return false
            val settings = Settings(context)
            settings.save(settings.load().copy(autoStart = true))
            context.startForegroundService(Intent(context, OpenMyGlassService::class.java))
            return true
        }

        fun stop(context: Context) {
            val settings = Settings(context)
            settings.save(settings.load().copy(autoStart = false))
            context.stopService(Intent(context, OpenMyGlassService::class.java))
        }
    }
}
