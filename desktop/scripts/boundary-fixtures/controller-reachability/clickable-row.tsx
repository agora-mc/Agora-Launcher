// Table rows were one of the real offenders in ModDetail's version pickers.
export function VersionRow({ onSelect }: { onSelect: () => void }) {
  return (
    <table><tbody>
      <tr onClick={onSelect}>
        <td>1.21.8</td>
      </tr>
    </tbody></table>
  );
}
