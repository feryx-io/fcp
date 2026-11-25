from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="RegistrationResponse")



@_attrs_define
class RegistrationResponse:
    """ Registration response.

        Attributes:
            aor (str):
            contact (str):
            expires_at (datetime.datetime):
            call_id (str):
            cseq (int):
            registered_at (datetime.datetime):
            expires_in_seconds (int):
            user_agent (None | str | Unset):
            q (float | None | Unset):
            node_id (None | str | Unset): The node that took the registration (its `[service] node_id`);
                null for one written before v0.25.639 that has not re-registered.
     """

    aor: str
    contact: str
    expires_at: datetime.datetime
    call_id: str
    cseq: int
    registered_at: datetime.datetime
    expires_in_seconds: int
    user_agent: None | str | Unset = UNSET
    q: float | None | Unset = UNSET
    node_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        aor = self.aor

        contact = self.contact

        expires_at = self.expires_at.isoformat()

        call_id = self.call_id

        cseq = self.cseq

        registered_at = self.registered_at.isoformat()

        expires_in_seconds = self.expires_in_seconds

        user_agent: None | str | Unset
        if isinstance(self.user_agent, Unset):
            user_agent = UNSET
        else:
            user_agent = self.user_agent

        q: float | None | Unset
        if isinstance(self.q, Unset):
            q = UNSET
        else:
            q = self.q

        node_id: None | str | Unset
        if isinstance(self.node_id, Unset):
            node_id = UNSET
        else:
            node_id = self.node_id


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "aor": aor,
            "contact": contact,
            "expires_at": expires_at,
            "call_id": call_id,
            "cseq": cseq,
            "registered_at": registered_at,
            "expires_in_seconds": expires_in_seconds,
        })
        if user_agent is not UNSET:
            field_dict["user_agent"] = user_agent
        if q is not UNSET:
            field_dict["q"] = q
        if node_id is not UNSET:
            field_dict["node_id"] = node_id

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        aor = d.pop("aor")

        contact = d.pop("contact")

        expires_at = datetime.datetime.fromisoformat(d.pop("expires_at"))




        call_id = d.pop("call_id")

        cseq = d.pop("cseq")

        registered_at = datetime.datetime.fromisoformat(d.pop("registered_at"))




        expires_in_seconds = d.pop("expires_in_seconds")

        def _parse_user_agent(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        user_agent = _parse_user_agent(d.pop("user_agent", UNSET))


        def _parse_q(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        q = _parse_q(d.pop("q", UNSET))


        def _parse_node_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        node_id = _parse_node_id(d.pop("node_id", UNSET))


        registration_response = cls(
            aor=aor,
            contact=contact,
            expires_at=expires_at,
            call_id=call_id,
            cseq=cseq,
            registered_at=registered_at,
            expires_in_seconds=expires_in_seconds,
            user_agent=user_agent,
            q=q,
            node_id=node_id,
        )


        registration_response.additional_properties = d
        return registration_response

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
