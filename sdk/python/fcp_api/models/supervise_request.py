from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="SuperviseRequest")



@_attrs_define
class SuperviseRequest:
    """ Who supervises, and who hears a whisper.

        Attributes:
            supervisor (str): A user's extension or name, or a SIP URI.
            heard_by (None | str | Unset): For a whisper: `a` (the caller) or `b` (the callee, the default).
     """

    supervisor: str
    heard_by: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        supervisor = self.supervisor

        heard_by: None | str | Unset
        if isinstance(self.heard_by, Unset):
            heard_by = UNSET
        else:
            heard_by = self.heard_by


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "supervisor": supervisor,
        })
        if heard_by is not UNSET:
            field_dict["heard_by"] = heard_by

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        supervisor = d.pop("supervisor")

        def _parse_heard_by(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        heard_by = _parse_heard_by(d.pop("heard_by", UNSET))


        supervise_request = cls(
            supervisor=supervisor,
            heard_by=heard_by,
        )


        supervise_request.additional_properties = d
        return supervise_request

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
