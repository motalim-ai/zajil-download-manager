package io.github.motalim_ai.zajil.browser

/** A tab, as the switcher lists it. */
data class TabInfo(val id: Long, val title: String, val url: String, val privateMode: Boolean)

/** Operations shared by the browser's regular and private tab groups. */
internal object BrowserTabs {
    fun inGroup(tabs: List<TabInfo>, privateMode: Boolean): List<TabInfo> =
        tabs.filter { it.privateMode == privateMode }

    fun lastInGroup(tabs: List<TabInfo>, privateMode: Boolean): TabInfo? =
        tabs.lastOrNull { it.privateMode == privateMode }

    fun needsPrivateConfirmation(tabs: List<TabInfo>, id: Long): Boolean =
        tabs.firstOrNull { it.id == id }?.let { tab ->
            tab.privateMode && tabs.count { it.privateMode } == 1
        } == true
}
