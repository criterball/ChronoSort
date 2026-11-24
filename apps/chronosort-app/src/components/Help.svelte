<script>
  const SECTIONS = [
    ["start", "Getting started"],
    ["browse", "Browsing, viewing and editing"],
    ["sort", "Sorting into year folders"],
    ["check", "Uncertain dates"],
    ["names", "File names"],
    ["dups", "Finding duplicates"],
    ["people", "People and faces"],
    ["places", "Adding a place"],
    ["safety", "How your photos are kept safe"],
    ["keys", "Shortcut keys"],
    ["ai", "About the AI"],
    ["about", "About ChronoSort"],
  ];
  const SHORTCUTS = [
    ["F1", "Open this help"],
    ["Esc  or  Ctrl+B", "Back to the File browser (the start screen)"],
    ["Ctrl+T", "Tools (sort, duplicates, people, places)"],
    ["Ctrl+1", "Sort photos into year folders"],
    ["Ctrl+2", "Find duplicates"],
    ["Ctrl+3", "People"],
    ["Ctrl+4", "Add a place"],
    ["Ctrl+,", "Settings"],
    ["Ctrl+Q", "Exit ChronoSort"],
    ["Ctrl+Enter", "Continue to the next step (the main blue button)"],
    ["Enter (in a year box)", "Confirm that year for the photo"],
    ["Tab / Shift+Tab", "Move between buttons and fields"],
    ["— File browser —", ""],
    ["Enter  or  double-click", "Open a folder, or view a photo full screen"],
    ["Backspace  /  Alt+←  /  Alt+→", "Up one folder / back / forward"],
    ["F2  /  F8", "Rename  /  New folder"],
    ["Ctrl+C, Ctrl+X, Ctrl+V", "Copy, cut, paste (never overwrites: a clash gets “(2)”)"],
    ["Del  /  Shift+Del", "Delete (Recycle Bin where the drive has one)  /  Delete permanently"],
    ["Ctrl+Z", "Undo the last change"],
    ["F5", "Refresh"],
    ["— Full-screen viewer —", ""],
    ["← →  or  mouse wheel", "Previous / next photo"],
    ["0  /  1  /  +  /  −", "Fit / actual size / zoom in / zoom out (Ctrl+wheel zooms too)"],
    ["L  R  H  V", "Rotate left / right, flip left–right / upside down"],
    ["X  /  Ctrl+R  /  Ctrl+S", "Crop / Resize / Save as"],
    ["I", "Information (EXIF), with an Edit button"],
    ["Esc", "Back to the browser"],
  ];
  const jump = (id) => document.getElementById("help-" + id)?.scrollIntoView({ behavior: "smooth", block: "start" });
</script>

<div class="layout">
  <nav class="toc" aria-label="Help topics">
    {#each SECTIONS as [id, title]}
      <button class="ghost" onclick={() => jump(id)}>{title}</button>
    {/each}
  </nav>

  <article class="text">
    <h1>Help</h1>

    <section id="help-start">
      <h2>Getting started</h2>
      <p>ChronoSort opens in the <strong>File browser</strong>: your drives and folders on the left, the files on the right. Look through your photos there, tidy up, and right-click a folder to sort it. The <strong>Tools</strong> button at the top has the big jobs:</p>
      <ul>
        <li><strong>Sort photos into year folders</strong> copies your photos and videos into folders named after the year they were taken, such as <span class="mono">2005</span> and <span class="mono">2006</span>.</li>
        <li><strong>Find duplicates</strong> finds files that are exactly the same and helps you tidy up the extra copies.</li>
        <li><strong>People</strong> finds the faces in your photos so you can name the people in them.</li>
      </ul>
      <p>You can go back to the browser at any time with the <strong>Browse</strong> button at the top, or the Esc key. The red <strong>Exit</strong> button closes ChronoSort.</p>
    </section>

    <section id="help-browse">
      <h2>Browsing, viewing and editing</h2>
      <p>Videos (MP4 and MOV from phones and cameras, including iPhone HEVC) get a thumbnail from ChronoSort's own built-in video reader, so no extra download is needed. Other kinds of video show a thumbnail only when the optional FFmpeg pack is installed.</p>
      <p>The browser shows <strong>all</strong> files, so nothing gets missed. Choose <em>Photos &amp; videos</em> at the top to see only those. The buttons <strong>S</strong>, <strong>M</strong> and <strong>L</strong> change the thumbnail size; Tiles, List and Details show more about each file.</p>
      <p><strong>Right-click a folder</strong> to sort it into years, find duplicates, people, or add a place. Right-click files for copy, move, rename and delete. Everything can be undone with Ctrl+Z, except <em>Delete permanently</em>.</p>
      <p><strong>Recycle Bin:</strong> ChronoSort checks every drive first. If the Recycle Bin is switched off for that drive, or it is a USB stick, memory card or network drive (which have none), only <em>Delete permanently</em> is offered, with a clear warning, because a “Recycle Bin” delete would really be permanent there.</p>
      <p><strong>Full-screen viewer:</strong> double-click a photo. Move the mouse to an edge of the screen: <em>left</em> = edit (rotate, flip, crop, resize, save), <em>right</em> = information (EXIF, with an Edit button), <em>top</em> = all photos of the folder, <em>bottom</em> = previous/next and zoom. Esc goes back.</p>
      <p><strong>Saving edits:</strong> by default a new copy is made next to the original. A JPEG that was only turned or flipped is saved without any quality loss. If you choose <em>Replace the original</em>, the original is kept in a hidden <span class="mono">.chronosort</span> folder next to it, so Ctrl+Z can bring it back. Changing information (date, title, keywords, place) keeps a safety copy the same way.</p>
    </section>

    <section id="help-sort">
      <h2>Sorting into year folders</h2>
      <ol>
        <li><strong>Folders</strong>: choose where your photos are now and where the sorted copy should go. Choose <em>Copy</em> (recommended) or <em>Move</em>.</li>
        <li><strong>Overview</strong>: ChronoSort reads every file (it changes nothing) and shows how many files it found, per year.</li>
        <li><strong>Check</strong>: some files have no reliable date, such as scans of old prints. You confirm the year for those.</li>
        <li><strong>File names</strong>: keep the original names, or choose a name with the date.</li>
        <li><strong>Final check</strong>: a preview of exactly what will happen. If your photos are already in month folders, you are asked whether to keep them.</li>
        <li><strong>Copy</strong>: the files are copied and checked. Afterwards you can open the folder or undo the whole run.</li>
      </ol>
      <p>Running it again later with the same folders only adds the new photos. Nothing is copied twice.</p>
    </section>

    <section id="help-check">
      <h2>Uncertain dates</h2>
      <p>The date a photo was taken is usually stored inside the file (EXIF). Scans, edited copies and files from messaging apps often lose it. ChronoSort then looks at other clues, like the file and folder names, and shows them as small labels. A crossed-out label is a clue it does not trust, for example a file date that only shows when the file was copied.</p>
      <p>Type a year and press Enter, or click the suggested year. For a whole folder of scans, use <strong>Set all</strong>. Files you leave open are simply not copied this time.</p>
      <p><strong>Click a picture</strong> to see it large, with every clue next to it. There, type the year and press Enter to confirm it and go straight to the next photo; <kbd>←</kbd> and <kbd>→</kbd> go back and forward, Esc closes.</p>
      <p><strong>Ask the AI</strong> (at the top of this step) lets the AI on this PC look at each open photo and suggest a year. Its guess appears as an amber “AI ≈ 1985 (1980–1990)” label; point at it to see why. If it reads a date printed in the picture, that shows as “Printed date”. The AI can be wrong, so its year is only a suggestion. It is used only if you click it or accept a folder's suggestions. <strong>Turn the AI off</strong> frees the graphics memory it uses; it also stops when you close ChronoSort.</p>
    </section>

    <section id="help-names">
      <h2>File names</h2>
      <p><strong>Keep the original names</strong> is the default. If two files in the same year have the same name, the second one gets <span class="mono">_1</span> at the end, the next <span class="mono">_2</span>, and so on. Nothing is ever overwritten.</p>
      <p>With <strong>My own pattern</strong> you build a name from pieces like year, month and the original name. ChronoSort checks the pattern as you type and shows examples from your own files.</p>
    </section>

    <section id="help-dups">
      <h2>Finding duplicates</h2>
      <p>Add one or more folders and click <strong>Find duplicates</strong>. Normally only files that are identical byte for byte count as duplicates.</p>
      <p><strong>Near-duplicates</strong> (tick the box before you start): ChronoSort also looks at every photo and finds the same photo made smaller or saved again in another quality, for example a copy that was e-mailed or exported from an old phone. These files are <em>not</em> identical, so such a set is left alone until you have compared the versions in the big preview (point at a version to see it) and ticked “include this set”. The version with the most pixels is suggested. Cropped photos and other shots of the same moment are not matched. Before anything moves, every version is checked again and is only touched if it is still exactly the file you compared.</p>
      <p>For each set of copies, ChronoSort selects the one to keep (the original, not a "copy of", and the oldest). You can pick another one. Then choose what happens to the extras:</p>
      <ul>
        <li><strong>Move them to a "duplicates" folder</strong> — recommended, and it can be undone.</li>
        <li><strong>Put them in the Recycle Bin</strong> — you are asked to confirm first.</li>
        <li><strong>Just save a list</strong> — a CSV file for Excel; nothing is changed.</li>
      </ul>
    </section>

    <section id="help-people">
      <h2>People and faces</h2>
      <p>Choose a folder and click <strong>Find faces</strong>. ChronoSort looks at every photo and puts faces that look alike into groups. The first time, it needs the small “Faces” AI part (Settings → AI models).</p>
      <ul>
        <li><strong>Name a group:</strong> type a name and press Enter. Next time, ChronoSort suggests that name for faces that look like this person: click <em>Yes, that's …</em> to confirm.</li>
        <li><strong>A face in the wrong group:</strong> click it (it gets a red frame), then <em>Split off</em>.</li>
        <li><strong>Two groups are the same person:</strong> tick both and click <em>Merge into one group</em>.</li>
        <li><strong>See the photo:</strong> double-click a face to show its photo in Explorer.</li>
      </ul>
      <p>Faces change over the years, so a person as a child and as an adult can end up in different groups. Name both groups with the same name: ChronoSort then knows both looks.</p>
      <p><strong>Privacy:</strong> everything happens on this PC. ChronoSort keeps the names, a small picture of each named face and a “fingerprint” of numbers per face, in its own folder. It never stores where your photos are, and it never changes them. <em>Forget</em> deletes a person and everything kept about them.</p>
    </section>

    <section id="help-places">
      <h2>Adding a place</h2>
      <p><strong>Add a place</strong> writes where photos were taken into many photos at once, so photo apps can show them on a map or find them by town. Choose a folder, tick the photos, type a town (for example <em>Adelaide</em>) and pick it from the list. The list of about 116,000 towns is built into ChronoSort, so no internet is needed.</p>
      <ul>
        <li>A <strong>town</strong> writes its map position and its name. A <strong>general area</strong> (for example <em>Tuscany</em> or <em>Queensland</em>) writes the area and country names, and <strong>country only</strong> writes just the country name; neither writes a made-up map position. When one of these replaces an older place, the old map position and town are removed so the photo does not name two places.</li>
        <li>Photos that already have a place are not ticked, so they are not changed by accident.</li>
        <li>This <strong>changes the photo files</strong>. Before each photo is changed, a copy is kept in <span class="mono">.chronosort\place-backups</span> inside the folder. <strong>Undo</strong> puts the photos back exactly as they were (unless you changed them again in the meantime).</li>
        <li>Best used on your sorted copies, not on your only originals.</li>
      </ul>
    </section>

    <section id="help-safety">
      <h2>How your photos are kept safe</h2>
      <ul>
        <li>In <strong>Copy</strong> mode your original files are only read, never changed.</li>
        <li>Every copy is <strong>checked</strong>: ChronoSort calculates a fingerprint while copying, reads the copy back and compares it.</li>
        <li>Copies keep their original <strong>dates</strong> (created and modified).</li>
        <li>Existing files are <strong>never overwritten</strong>.</li>
        <li>Every run can be <strong>undone</strong>. Copies you have changed since are kept.</li>
        <li>In <strong>Move</strong> mode an original only goes to the Recycle Bin after its copy has been checked.</li>
        <li>If the PC switches off in the middle, nothing is lost: the next run carries on.</li>
        <li>Your photos and videos <strong>never leave your PC</strong>, and no online AI is used. The internet is only used when you choose to download an AI model, and then only the model is downloaded.</li>
      </ul>
    </section>

    <section id="help-keys">
      <h2>Shortcut keys</h2>
      <table>
        <tbody>
          {#each SHORTCUTS as [k, what]}
            {#if what === ""}<tr><th colspan="2" class="grp">{k.replace(/—/g, "").trim()}</th></tr>{:else}<tr><td><kbd>{k}</kbd></td><td>{what}</td></tr>{/if}
          {/each}
        </tbody>
      </table>
    </section>

    <section id="help-ai">
      <h2>About the AI</h2>
      <p>ChronoSort dates photos from the information inside the files first, without AI. Two AI helpers run entirely on your own PC:</p>
      <ul>
        <li><strong>Faces</strong> (People): you name a person once, and ChronoSort recognises them in other photos.</li>
        <li><strong>Reading the photo</strong> (Ask the AI, when checking uncertain dates): a vision model reads dates printed on old prints and looks at clues like the kind of print, clothes or cars, and suggests a year.</li>
      </ul>
      <p><strong>Settings → AI models</strong> checks this PC's memory and graphics card and suggests the models that fit (Max, Standard, Light, or without AI). Click <strong>Download</strong> to get a model; it comes from the model's official publisher and is checked against its fingerprint (SHA-256) before it is used. A download can be cancelled and continues later where it stopped. No internet on the PC? Copy the files from a USB stick and choose them with “Use a file I have…”. ChronoSort never sends your photos anywhere.</p>
    </section>

    <section id="help-about">
      <h2>About ChronoSort</h2>
      <p>Free software under the GNU General Public License, version 3 or later: you may use, share and change it, and changed versions must stay free too. It comes with no warranty — keep a backup of photos that matter.</p>
    </section>
  </article>
</div>

<style>
  .grp { text-align: left; padding-top: 14px; font-size: 15px; }
  .layout { display: grid; grid-template-columns: 230px 1fr; gap: 28px; max-width: 1100px; margin: 0 auto; padding: 24px; }
  .toc { position: sticky; top: 16px; align-self: start; display: grid; gap: 2px; }
  .toc button { text-align: left; padding: 8px 12px; }
  .text { display: grid; gap: 26px; max-width: 760px; font-size: 15px; line-height: 1.6; }
  .text h1 { font-size: 26px; }
  section { display: grid; gap: 8px; scroll-margin-top: 12px; }
  h2 { font-size: 19px; }
  ul, ol { margin: 0; padding-left: 22px; display: grid; gap: 4px; }
  table { border-collapse: collapse; }
  td { padding: 7px 14px 7px 0; border-bottom: 1px solid var(--border); vertical-align: top; }
  kbd { font: 13px var(--mono); border: 1px solid var(--border); border-bottom-width: 2px; border-radius: 5px; padding: 2px 8px; background: var(--surface); white-space: pre; }
</style>
