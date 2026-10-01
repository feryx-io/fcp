from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="TransferRequest")



@_attrs_define
class TransferRequest:
    """ Where a party is transferred.

        Attributes:
            target (str): A user, an extension or a number, routed as a call to it would be.
            party (None | str | Unset): Who is transferred: `caller` (the default) or `callee`. The other
                party is dropped once the target answers.
     """

    target: str
    party: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        target = self.target

        party: None | str | Unset
        if isinstance(self.party, Unset):
            party = UNSET
        else:
            party = self.party


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "target": target,
        })
        if party is not UNSET:
            field_dict["party"] = party

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        target = d.pop("target")

        def _parse_party(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        party = _parse_party(d.pop("party", UNSET))


        transfer_request = cls(
            target=target,
            party=party,
        )


        transfer_request.additional_properties = d
        return transfer_request

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
