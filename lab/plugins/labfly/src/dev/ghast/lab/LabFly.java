package dev.ghast.lab;

import java.util.List;

import org.bukkit.Bukkit;
import org.bukkit.GameMode;
import org.bukkit.command.Command;
import org.bukkit.command.CommandSender;
import org.bukkit.entity.Player;
import org.bukkit.event.EventHandler;
import org.bukkit.event.Listener;
import org.bukkit.event.player.PlayerChangedWorldEvent;
import org.bukkit.event.player.PlayerGameModeChangeEvent;
import org.bukkit.event.player.PlayerJoinEvent;
import org.bukkit.event.player.PlayerRespawnEvent;
import org.bukkit.plugin.java.JavaPlugin;

/**
 * Lab-only /fly: lets survival test accounts keep allowFlight, as many survival servers grant it.
 * Built against the Bukkit API shared by 1.8.8 and 1.16.5 (Java 8 bytecode).
 */
public final class LabFly extends JavaPlugin implements Listener {

    @Override
    public void onEnable() {
        saveDefaultConfig();
        getServer().getPluginManager().registerEvents(this, this);
    }

    private boolean autoFly(Player p) {
        List<String> names = getConfig().getStringList("auto-fly");
        for (String n : names) {
            if (n.equalsIgnoreCase(p.getName())) return true;
        }
        return false;
    }

    /** One tick later, so vanilla's own ability resets (respawn, gamemode) do not undo it. */
    private void grantNextTick(final Player p) {
        if (!autoFly(p)) return;
        Bukkit.getScheduler().runTask(this, new Runnable() {
            @Override
            public void run() {
                if (p.isOnline() && p.getGameMode() == GameMode.SURVIVAL) p.setAllowFlight(true);
            }
        });
    }

    @EventHandler public void onJoin(PlayerJoinEvent e) { grantNextTick(e.getPlayer()); }
    @EventHandler public void onRespawn(PlayerRespawnEvent e) { grantNextTick(e.getPlayer()); }
    @EventHandler public void onWorld(PlayerChangedWorldEvent e) { grantNextTick(e.getPlayer()); }
    @EventHandler public void onGameMode(PlayerGameModeChangeEvent e) { grantNextTick(e.getPlayer()); }

    @Override
    public boolean onCommand(CommandSender sender, Command cmd, String label, String[] args) {
        Player target;
        if (args.length >= 1) {
            target = Bukkit.getPlayerExact(args[0]);
        } else if (sender instanceof Player) {
            target = (Player) sender;
        } else {
            sender.sendMessage("Usage: /fly <player> [on|off]");
            return true;
        }
        if (target == null) {
            sender.sendMessage("Player not online");
            return true;
        }
        boolean on = args.length >= 2 ? args[1].equalsIgnoreCase("on") : !target.getAllowFlight();
        target.setAllowFlight(on);
        if (!on) target.setFlying(false);
        sender.sendMessage("Fly " + (on ? "enabled" : "disabled") + " for " + target.getName());
        return true;
    }
}
